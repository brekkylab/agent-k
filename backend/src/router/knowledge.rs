//! HTTP API for a workspace's knowledge index membership.
//!
//! Membership is a set of references under `/files/knowledge`: each `*.ref` file
//! holds the unified-tree path of a target (a local file, a mounted object, or a
//! directory). Writing / deleting a reference goes through
//! [`WorkspaceFs`](::workspace::WorkspaceFs), whose `/files/knowledge` hook spawns
//! a resync — so these routes never touch the resyncer directly except for the
//! explicit `resync`.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use bytes::Bytes;
use futures_util::StreamExt as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ::workspace::{FsError as WsFsError, OpenOptions, WorkspaceFs};

use crate::auth::AuthUser;
use crate::state::{AppState, KNOWLEDGE_DIR, KnowledgeRef, REF_SUFFIX};

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

    let fs = state
        .workspaces
        .get_fs(wid)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;

    match fs.metadata(&target).await {
        Ok(_) => {}
        Err(WsFsError::NotFound) => return Err(err(StatusCode::NOT_FOUND, "target not found")),
        Err(_) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")),
    }

    match fs.create_dir(KNOWLEDGE_DIR).await {
        Ok(()) | Err(WsFsError::Exists) => {}
        Err(_) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")),
    }

    let name = ref_name_for(&target);
    let ref_path = format!("{KNOWLEDGE_DIR}/{name}");
    let body = KnowledgeRef {
        path: target.clone(),
    }
    .to_bytes();
    write_ref(&fs, &ref_path, body)
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
    let fs = state
        .workspaces
        .get_fs(wid)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;

    let mut items = Vec::new();
    match fs.read_dir(KNOWLEDGE_DIR).await {
        Ok(mut stream) => {
            while let Some(entry) = stream.next().await {
                let Ok(entry) = entry else { continue };
                let name = String::from_utf8_lossy(&entry.name()).into_owned();
                if !name.ends_with(REF_SUFFIX) {
                    continue;
                }
                let ref_path = format!("{KNOWLEDGE_DIR}/{name}");
                let Ok(bytes) = read_all(&fs, &ref_path).await else {
                    continue;
                };
                if let Ok(r) = serde_json::from_slice::<KnowledgeRef>(&bytes) {
                    items.push(RefResponse {
                        name,
                        target_path: r.path,
                    });
                }
            }
        }
        Err(WsFsError::NotFound) => {}
        Err(_) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")),
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
    let fs = state
        .workspaces
        .get_fs(wid)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?;
    match fs.remove_file(&format!("{KNOWLEDGE_DIR}/{name}")).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(WsFsError::NotFound) => Err(err(StatusCode::NOT_FOUND, "reference not found")),
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

async fn write_ref(fs: &WorkspaceFs, ref_path: &str, body: Vec<u8>) -> Result<(), WsFsError> {
    let mut file = fs
        .open(
            ref_path,
            OpenOptions {
                write: true,
                create: true,
                truncate: true,
                ..Default::default()
            },
        )
        .await?;
    file.write_bytes(Bytes::from(body)).await?;
    file.flush().await
}

async fn read_all(fs: &WorkspaceFs, path: &str) -> Result<Vec<u8>, WsFsError> {
    let mut file = fs
        .open(
            path,
            OpenOptions {
                read: true,
                ..Default::default()
            },
        )
        .await?;
    let mut out = Vec::new();
    loop {
        let chunk = file.read_bytes(256 * 1024).await?;
        if chunk.is_empty() {
            break;
        }
        out.extend_from_slice(&chunk);
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
