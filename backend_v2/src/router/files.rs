//! Browser-facing JSON file API for a workspace's unified tree (local files +
//! external mounts). A thin adapter over [`WorkspaceFs`](crate::state) — the
//! same core the WebDAV layer wraps — for SPA consumption:
//!
//! * `GET /workspaces/{wid}/tree?path=/notion` → JSON directory listing.
//! * `GET /workspaces/{wid}/file?path=/notion/pages/…/page.json` → file bytes.
//!
//! Read-only. Every route is gated by [`require_owned_workspace`], and `path`
//! is rejected if it contains a `..` traversal segment.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    state::{AppState, FsError, OpenOptions, ReadDirMeta},
};

use super::{
    error::{ApiError, err},
    workspace::require_owned_workspace,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathQuery {
    /// Workspace-relative path (e.g. `/notion/pages`). Defaults to the root.
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

    let mut stream = fs.read_dir(&path, ReadDirMeta::None).await.map_err(fs_err)?;
    let mut items = Vec::new();
    while let Some(entry) = stream.next().await {
        let entry = entry.map_err(fs_err)?;
        let (is_dir, size, mtime) = match entry.metadata() {
            Ok(st) => (st.is_dir(), st.len, st.modified.and_then(epoch_secs)),
            Err(_) => (false, 0, None),
        };
        items.push(TreeEntry {
            name: String::from_utf8_lossy(&entry.name()).into_owned(),
            is_dir,
            size,
            mtime,
        });
    }
    // Directories first, then case-sensitive name order — a stable UI ordering.
    items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(Json(TreeResponse { path, items }))
}

/// `GET /workspaces/{wid}/file?path=…` — stream a file's bytes with a
/// best-effort `Content-Type`. Registered as a plain route (binary body).
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

    let meta = fs.metadata(&path).await.map_err(fs_err)?;
    if meta.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "path is a directory"));
    }

    let mut file = fs
        .open(
            &path,
            OpenOptions {
                read: true,
                ..Default::default()
            },
        )
        .await
        .map_err(fs_err)?;
    let mut buf: Vec<u8> = Vec::with_capacity(meta.len.min(1 << 20) as usize);
    loop {
        let chunk = file.read_bytes(64 * 1024).await.map_err(fs_err)?;
        if chunk.is_empty() {
            break;
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(([(header::CONTENT_TYPE, content_type(&path))], buf).into_response())
}

/// Reject `..` traversal and normalise to a leading-slash, workspace-relative
/// path. The JSON API bypasses `dav_server`'s path handling, so this guard is
/// what keeps a request inside the workspace.
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

/// Seconds since the Unix epoch (signed; pre-epoch times clamp via `as i64`).
fn epoch_secs(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

/// Map a filesystem error onto an HTTP status.
fn fs_err(e: FsError) -> ApiError {
    match e {
        FsError::NotFound => err(StatusCode::NOT_FOUND, "not found"),
        FsError::Forbidden => err(StatusCode::FORBIDDEN, "forbidden"),
        FsError::Exists => err(StatusCode::CONFLICT, "already exists"),
        FsError::NotImplemented => err(StatusCode::NOT_IMPLEMENTED, "not supported"),
        FsError::GeneralFailure => {
            err(StatusCode::INTERNAL_SERVER_ERROR, "filesystem error")
        }
    }
}
