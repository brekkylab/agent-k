//! Browser-facing JSON file API over a workspace's unified tree — the same
//! [`ForwardFs`] view the guest sees at `/mnt/workspace` (local files under
//! `files/`, each provider mount as a sibling).
//!
//! * `GET /workspaces/{wid}/tree?path=…` → JSON directory listing.
//! * `GET /workspaces/{wid}/file?path=…` → file bytes.
//!
//! Read-only; gated by [`require_owned_workspace`]. `path` rejects `..`.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{auth::AuthUser, state::AppState, vfs::ForwardFs};

use super::{
    error::{ApiError, err},
    workspace::require_owned_workspace,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathQuery {
    /// Workspace-relative path; defaults to the root.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Modification time, seconds since the Unix epoch, if the source reports it.
    pub mtime: Option<i64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeResponse {
    /// The (normalised) directory that was listed.
    pub path: String,
    pub items: Vec<TreeEntry>,
}

/// `GET /workspaces/{wid}/tree?path=…` — list a directory in the unified tree.
pub(super) async fn list_tree(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(wid): Path<Uuid>,
    Query(q): Query<PathQuery>,
) -> Result<Json<TreeResponse>, ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    let path = normalize_path(q.path.as_deref().unwrap_or("/"))?;
    let fs = state.workspaces.get_fs(wid).await?;

    let st = fs.stat(&path).await.map_err(internal)?;
    if !st.exists {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }
    if !st.is_dir {
        return Err(err(StatusCode::BAD_REQUEST, "not a directory"));
    }

    let mut items: Vec<TreeEntry> = fs
        .readdir(&path)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|e| TreeEntry {
            name: e.name,
            is_dir: e.is_dir,
            size: e.size,
            mtime: e.mtime.map(|s| s as i64),
        })
        .collect();
    // Directories first, then name order — a stable UI ordering.
    items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(Json(TreeResponse { path, items }))
}

/// `GET /workspaces/{wid}/file?path=…` — file bytes with a best-effort
/// `Content-Type`, read in a single call (no chunking).
pub(super) async fn read_file(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(wid): Path<Uuid>,
    Query(q): Query<PathQuery>,
) -> Result<Response, ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    let path = match q.path.as_deref() {
        Some(p) if !p.trim().trim_matches('/').is_empty() => normalize_path(p)?,
        _ => return Err(err(StatusCode::BAD_REQUEST, "path query param is required")),
    };
    let fs = state.workspaces.get_fs(wid).await?;

    let st = fs.stat(&path).await.map_err(internal)?;
    if !st.exists {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }
    if st.is_dir {
        return Err(err(StatusCode::BAD_REQUEST, "path is a directory"));
    }

    let bytes = fs.read(&path, None, None).await.map_err(internal)?;
    Ok(([(header::CONTENT_TYPE, content_type(&path))], bytes).into_response())
}

/// Reject `..` traversal and normalise to a leading-slash, workspace-relative
/// path — the only guard keeping a request inside the workspace.
fn normalize_path(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim();
    if trimmed.split('/').any(|seg| seg == "..") {
        return Err(err(StatusCode::BAD_REQUEST, "invalid path"));
    }
    let rel = trimmed.trim_start_matches('/');
    Ok(if rel.is_empty() {
        "/".to_string()
    } else {
        format!("/{rel}")
    })
}

/// Best-effort `Content-Type` from the file extension; `application/octet-stream`
/// when unknown so the browser downloads rather than mis-renders.
fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "json" => "application/json; charset=utf-8",
        "txt" | "log" => "text/plain; charset=utf-8",
        "md" | "markdown" => "text/markdown; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Map a filesystem/provider error onto a 500.
fn internal(e: anyhow::Error) -> ApiError {
    tracing::error!("files api: {e:#}");
    err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
