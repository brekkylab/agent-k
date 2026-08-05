//! HTTP API for a workspace's knowledge index membership.
//!
//! Membership is a set of references under `/files/knowledge`: each `*.ref` file
//! holds the unified-tree path of a target (a local file, a mounted object, or a
//! directory). Writing / deleting a reference goes through the workspace's cortex
//! [`Workspace`](cortex::Workspace), whose `files/knowledge` hook spawns a resync
//! — so these routes never touch the resyncer directly except for the explicit
//! `resync`.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use cortex::{CortexError, FileExt, FileHandle, Mountable, OpenOptions, Workspace};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::state::{AppState, KNOWLEDGE_ROOT, KnowledgeRef, REF_SUFFIX};

use super::{error::ApiError, error::err, workspace::require_owned_workspace};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRefRequest {
    /// Absolute unified-tree path of the target to index, e.g. `/files/docs/q3.pdf`
    /// (local) or `/s3-prod/reports/q3.pdf` (mounted). May point at a directory.
    pub target_path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RefResponse {
    pub name: String,
    pub target_path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RefListResponse {
    pub items: Vec<RefResponse>,
}

/// A workspace-relative path (leading slash) as a cortex mount path (no leading
/// slash — cortex treats a request path as relative to the workspace root).
fn cx(path: &str) -> PathBuf {
    PathBuf::from(path.trim_start_matches('/'))
}

/// `POST /workspaces/{wid}/knowledge/refs` — reference a target into knowledge.
pub(super) async fn create_ref(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(wid): Path<Uuid>,
    Json(payload): Json<CreateRefRequest>,
) -> Result<(StatusCode, Json<RefResponse>), ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;

    let target = clean_target(&payload.target_path).map_err(|m| err(StatusCode::BAD_REQUEST, m))?;
    if is_under_knowledge(&target) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "cannot reference the knowledge directory itself",
        ));
    }

    let ws = state
        .workspaces
        .cortex_workspace(wid)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;

    let name = ref_name_for(&target);
    let ref_path = format!("{KNOWLEDGE_ROOT}/{name}");
    let body = KnowledgeRef {
        path: target.clone(),
    }
    .to_bytes();

    // Writing the `.ref` fires the workspace's knowledge hook, which spawns the
    // resync. A missing target is a 404; anything else is a 500.
    match ws.stat(&cx(&target)).await {
        Ok(_) => {}
        Err(CortexError::NotFound) => return Err(err(StatusCode::NOT_FOUND, "target not found")),
        Err(_) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")),
    }
    match ws.mkdir(&cx(KNOWLEDGE_ROOT)).await {
        Ok(()) | Err(CortexError::AlreadyExists) => {}
        Err(_) => {
            return Err(err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to write reference",
            ));
        }
    }
    write_ref(&ws, &ref_path, &body)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "failed to write reference"))?;

    Ok((
        StatusCode::CREATED,
        Json(RefResponse {
            name,
            target_path: target,
        }),
    ))
}

/// `GET /workspaces/{wid}/knowledge/refs` — list the workspace's references.
pub(super) async fn list_refs(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(wid): Path<Uuid>,
) -> Result<Json<RefListResponse>, ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    let ws = state
        .workspaces
        .cortex_workspace(wid)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;

    let entries = match ws.list(&cx(KNOWLEDGE_ROOT)).await {
        Ok(e) => e,
        Err(CortexError::NotFound) => Vec::new(),
        Err(_) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")),
    };
    let mut items = Vec::new();
    for entry in entries {
        let name = entry.name;
        if !name.ends_with(REF_SUFFIX) {
            continue;
        }
        let ref_path = format!("{KNOWLEDGE_ROOT}/{name}");
        let Ok(bytes) = read_all(&ws, &ref_path).await else {
            continue;
        };
        if let Ok(r) = serde_json::from_slice::<KnowledgeRef>(&bytes) {
            items.push(RefResponse {
                name,
                target_path: r.path,
            });
        }
    }

    Ok(Json(RefListResponse { items }))
}

/// `DELETE /workspaces/{wid}/knowledge/refs/{name}` — drop a reference.
pub(super) async fn delete_ref(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path((wid, name)): Path<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    if name.contains('/') || name.contains("..") || !name.ends_with(REF_SUFFIX) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid reference name"));
    }
    let ws = state
        .workspaces
        .cortex_workspace(wid)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;
    let ref_path = format!("{KNOWLEDGE_ROOT}/{name}");
    match ws.unlink(&cx(&ref_path)).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(CortexError::NotFound) => Err(err(StatusCode::NOT_FOUND, "reference not found")),
        Err(_) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")),
    }
}

/// `POST /workspaces/{wid}/knowledge/resync` — force a resync now.
pub(super) async fn resync(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(wid): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    state.workspaces.resyncer().spawn_resync(wid);
    Ok(StatusCode::ACCEPTED)
}

/// Validate + normalise a reference target: an absolute unified-tree path of
/// normal segments (no relative, no `.`/`..`).
fn clean_target(raw: &str) -> Result<String, &'static str> {
    let t = raw.trim();
    if !t.starts_with('/') {
        return Err("target_path must be an absolute unified-tree path");
    }
    let segments: Vec<&str> = t.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err("target_path must not be the root");
    }
    if segments.iter().any(|s| *s == "." || *s == "..") {
        return Err("target_path must not contain '.' or '..'");
    }
    Ok(format!("/{}", segments.join("/")))
}

/// True when `path` is `/files/knowledge` or lives under it.
fn is_under_knowledge(path: &str) -> bool {
    let p = path.trim_start_matches('/');
    p == "files/knowledge" || p.starts_with("files/knowledge/")
}

/// A readable, unique reference file name derived from the target's basename.
fn ref_name_for(target: &str) -> String {
    let base = target.rsplit('/').next().unwrap_or("item");
    let stem = base.split('.').next().unwrap_or(base);
    let slug: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "item" } else { slug };
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{slug}-{}{REF_SUFFIX}", &suffix[..8])
}

/// Write `body` to `ref_path`, truncating.
async fn write_ref(ws: &Workspace, ref_path: &str, body: &[u8]) -> anyhow::Result<()> {
    let (handle, _) = ws
        .open(
            &cx(ref_path),
            OpenOptions::write_only().create(true).truncate(true),
        )
        .await?;
    handle.write_all_at(body, 0).await?;
    handle.flush().await?;
    Ok(())
}

/// Read an entire file into memory.
async fn read_all(ws: &Workspace, path: &str) -> anyhow::Result<Vec<u8>> {
    let (handle, stat) = ws
        .open(
            &cx(path),
            OpenOptions::read_only(),
        )
        .await?;
    let mut out = vec![0u8; stat.size as usize];
    if stat.size > 0 {
        handle.read_exact_at(&mut out, 0).await?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::clean_target;

    #[test]
    fn clean_target_requires_absolute_normal_path() {
        assert_eq!(clean_target("/files/a.md").unwrap(), "/files/a.md");
        assert_eq!(clean_target("  /a//b/ ").unwrap(), "/a/b");
        assert!(clean_target("a.md").is_err());
        assert!(clean_target("/").is_err());
        assert!(clean_target("/../etc/passwd").is_err());
        assert!(clean_target("/a/./b").is_err());
    }
}
