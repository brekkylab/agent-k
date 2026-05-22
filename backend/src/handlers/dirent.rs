use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use aide::NoApi;
use axum::{
    Extension, Json,
    extract::{Multipart, Path as AxumPath, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiResult, AppError},
    model::{
        Dirent, DirentBatchOp, DirentBatchResult, DirentKind, DirentScopeQuery, FailedFile,
        ListResponse,
    },
    state::AppState,
};

/// Validate and join a relative path onto a root directory.
///
/// Rejects: empty strings, absolute paths, `..` segments, NUL bytes,
/// and paths that normalize to empty.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.is_empty() {
        return Err("path must not be empty".into());
    }
    if rel.contains('\0') {
        return Err("path must not contain NUL bytes".into());
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(rel).components() {
        match component {
            Component::ParentDir => return Err("path must not contain '..' segments".into()),
            Component::RootDir => return Err("path must not be absolute".into()),
            Component::Prefix(_) => return Err("path must not contain a drive prefix".into()),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err("path is empty after normalization".into());
    }

    Ok(root.join(normalized))
}

fn has_path_prefix(rel: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return true;
    }
    rel == prefix || rel.starts_with(&format!("{prefix}/"))
}

// ── Scope types and helpers ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum DirentScope {
    Shared { project_id: Uuid },
    Inputs { project_id: Uuid, session_id: Uuid },
    Artifacts { project_id: Uuid, session_id: Uuid },
}

impl DirentScope {
    fn project_id(&self) -> Uuid {
        match self {
            Self::Shared { project_id }
            | Self::Inputs { project_id, .. }
            | Self::Artifacts { project_id, .. } => *project_id,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Shared { project_id: a }, Self::Shared { project_id: b }) => a == b,
            (
                Self::Inputs {
                    project_id: a,
                    session_id: sa,
                },
                Self::Inputs {
                    project_id: b,
                    session_id: sb,
                },
            ) => a == b && sa == sb,
            (
                Self::Artifacts {
                    project_id: a,
                    session_id: sa,
                },
                Self::Artifacts {
                    project_id: b,
                    session_id: sb,
                },
            ) => a == b && sa == sb,
            _ => false,
        }
    }

    fn prefix_str(&self) -> String {
        match self {
            Self::Shared { project_id } => format!("projects/{}/shared", project_id),
            Self::Inputs {
                project_id,
                session_id,
            } => format!("projects/{}/sessions/{}/inputs", project_id, session_id),
            Self::Artifacts {
                project_id,
                session_id,
            } => format!("projects/{}/sessions/{}/artifacts", project_id, session_id),
        }
    }
}

pub(crate) struct ParsedDirentPath {
    pub scope: DirentScope,
    pub tail: PathBuf,
}

pub(crate) fn parse_dirent_path(path: &str) -> Result<ParsedDirentPath, crate::error::ApiError> {
    if path.contains('\0') {
        return Err(AppError::bad_request("path must not contain NUL bytes"));
    }
    let path = path.trim_matches('/').replace('\\', "/");
    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    for c in &components {
        if *c == ".." || *c == "." {
            return Err(AppError::bad_request("path must not contain '..' segments"));
        }
    }
    if components.len() < 3 || components[0] != "projects" {
        return Err(AppError::bad_request(
            "path must start with 'projects/{uuid}/shared|sessions'",
        ));
    }
    let project_id = Uuid::parse_str(components[1])
        .map_err(|_| AppError::bad_request("invalid project id in path"))?;

    let (scope, tail_start) = if components[2] == "shared" {
        (DirentScope::Shared { project_id }, 3)
    } else if components[2] == "sessions" {
        if components.len() < 5 {
            return Err(AppError::bad_request(
                "session path requires: sessions/{uuid}/inputs|artifacts",
            ));
        }
        let session_id = Uuid::parse_str(components[3])
            .map_err(|_| AppError::bad_request("invalid session id in path"))?;
        let scope = match components[4] {
            "inputs" => DirentScope::Inputs {
                project_id,
                session_id,
            },
            "artifacts" => DirentScope::Artifacts {
                project_id,
                session_id,
            },
            _ => {
                return Err(AppError::bad_request(
                    "unknown scope kind; expected 'inputs' or 'artifacts'",
                ));
            }
        };
        (scope, 5)
    } else {
        return Err(AppError::bad_request(
            "expected 'shared' or 'sessions' after project id",
        ));
    };

    let tail: PathBuf = components[tail_start..].iter().collect();
    Ok(ParsedDirentPath { scope, tail })
}

pub(crate) fn scope_root(state: &AppState, scope: &DirentScope) -> PathBuf {
    match scope {
        DirentScope::Shared { project_id } => state
            .data_root
            .join("projects")
            .join(project_id.to_string())
            .join("shared"),
        DirentScope::Inputs {
            project_id,
            session_id,
        } => state
            .data_root
            .join("projects")
            .join(project_id.to_string())
            .join("sessions")
            .join(session_id.to_string())
            .join("inputs"),
        DirentScope::Artifacts {
            project_id,
            session_id,
        } => state
            .data_root
            .join("projects")
            .join(project_id.to_string())
            .join("sessions")
            .join(session_id.to_string())
            .join("artifacts"),
    }
}

pub(crate) async fn enforce_scope_access(
    state: &AppState,
    auth_user: &AuthUser,
    scope: &DirentScope,
    write: bool,
) -> ApiResult<()> {
    use crate::repository::SessionAccess;

    let in_project = state
        .repository
        .user_in_project(auth_user.id, scope.project_id())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }
    match scope {
        DirentScope::Inputs {
            project_id,
            session_id,
        }
        | DirentScope::Artifacts {
            project_id,
            session_id,
        } => {
            let result = state
                .repository
                .get_session_with_authz(*session_id, auth_user.id)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?;
            let (session, access) =
                result.ok_or_else(|| AppError::not_found("session not found or access denied"))?;
            if session.project_id != *project_id {
                return Err(AppError::not_found("session not in project"));
            }
            if write && matches!(access, SessionAccess::ReadOnlyMember) {
                return Err(AppError::forbidden("read-only access to this session"));
            }
        }
        DirentScope::Shared { .. } => {}
    }
    Ok(())
}

/// POST /dirents?path=<scope_root>
pub async fn dirent_upload(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Query(scope_q): Query<DirentScopeQuery>,
    NoApi(mut multipart): NoApi<Multipart>,
) -> ApiResult<Json<DirentBatchResult>> {
    let parsed = parse_dirent_path(&scope_q.path)?;
    if !parsed.tail.as_os_str().is_empty() {
        return Err(AppError::bad_request(
            "upload path must be a scope root (no file suffix)",
        ));
    }
    enforce_scope_access(&state, &auth_user, &parsed.scope, true).await?;
    let root = scope_root(&state, &parsed.scope);
    let scope_prefix = parsed.scope.prefix_str();
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| AppError::internal(format!("failed to create directory: {e}")))?;

    let max_bytes = state.max_upload_bytes;
    let mut succeeded: Vec<Dirent> = Vec::new();
    let mut failed: Vec<FailedFile> = Vec::new();

    'files: while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?
    {
        let filename = match field.file_name() {
            Some(name) => name.to_string(),
            None => {
                failed.push(FailedFile {
                    path: String::new(),
                    error: "missing filename".into(),
                });
                continue;
            }
        };

        let host_path = match safe_join(&root, &filename) {
            Ok(p) => p,
            Err(e) => {
                failed.push(FailedFile {
                    path: filename,
                    error: e,
                });
                continue;
            }
        };

        let parent = match host_path.parent() {
            Some(p) => p,
            None => {
                failed.push(FailedFile {
                    path: filename,
                    error: "invalid path".into(),
                });
                continue;
            }
        };
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            failed.push(FailedFile {
                path: filename,
                error: format!("failed to create dirs: {e}"),
            });
            continue;
        }

        // Atomic write: stream chunks directly to temp file; track byte_count for size limit.
        let tmp_path = root.join(format!(".tmp.{}", Uuid::new_v4().simple()));
        let mut tmp_file = match tokio::fs::File::create(&tmp_path).await {
            Ok(f) => f,
            Err(e) => {
                failed.push(FailedFile {
                    path: filename,
                    error: format!("failed to create temp file: {e}"),
                });
                continue;
            }
        };

        let mut byte_count: u64 = 0;
        loop {
            match field
                .chunk()
                .await
                .map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?
            {
                Some(chunk) => {
                    byte_count += chunk.len() as u64;
                    if byte_count > max_bytes as u64 {
                        // Drain the rest so the multipart stream stays parseable.
                        loop {
                            match field.chunk().await {
                                Ok(Some(_)) => {}
                                _ => break,
                            }
                        }
                        drop(tmp_file);
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                        failed.push(FailedFile {
                            path: filename,
                            error: format!("file exceeds maximum size ({max_bytes} bytes)"),
                        });
                        continue 'files;
                    }
                    if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut tmp_file, &chunk).await
                    {
                        drop(tmp_file);
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                        failed.push(FailedFile {
                            path: filename,
                            error: format!("failed to write chunk: {e}"),
                        });
                        continue 'files;
                    }
                }
                None => break,
            }
        }
        drop(tmp_file);

        if let Err(e) = tokio::fs::rename(&tmp_path, &host_path).await {
            if let Err(rm_e) = tokio::fs::remove_file(&tmp_path).await {
                tracing::warn!(path = %tmp_path.display(), "failed to remove orphaned temp file: {rm_e}");
            }
            failed.push(FailedFile {
                path: filename,
                error: format!("failed to finalize file: {e}"),
            });
            continue;
        }

        let modified_at = tokio::fs::metadata(&host_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .map(DateTime::<Utc>::from);
        succeeded.push(Dirent {
            path: format!("{scope_prefix}/{filename}"),
            kind: DirentKind::File,
            bytes: Some(byte_count),
            modified_at,
        });
    }

    tracing::info!(path = %scope_q.path, count = %succeeded.len(), "dirents uploaded");

    Ok(Json(DirentBatchResult { succeeded, failed }))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DirentListQuery {
    path: String,
    prefix: Option<String>,
    recursive: Option<bool>,
}

/// GET /dirents?path=<scope_root>[&prefix=...][&recursive=...]
pub async fn dirent_list(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<DirentListQuery>,
) -> ApiResult<Json<ListResponse>> {
    let parsed = parse_dirent_path(&query.path)?;
    if !parsed.tail.as_os_str().is_empty() {
        return Err(AppError::bad_request(
            "list path must be a scope root (no file suffix)",
        ));
    }
    enforce_scope_access(&state, &auth_user, &parsed.scope, false).await?;
    let root = scope_root(&state, &parsed.scope);
    let scope_prefix = parsed.scope.prefix_str();

    // Handle NotFound gracefully
    match tokio::fs::metadata(&root).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(ListResponse { entries: vec![] }));
        }
        Err(e) => return Err(AppError::internal(e.to_string())),
        Ok(_) => {}
    }

    let recursive = query.recursive.unwrap_or(true);
    let mut entries: Vec<Dirent> = Vec::new();
    let mut queue: Vec<PathBuf> = vec![root.clone()];

    while let Some(dir) = queue.pop() {
        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) => {
                tracing::warn!(path = %dir.display(), "read_dir error: {e}");
                continue;
            }
        };

        loop {
            let entry = match read_dir.next_entry().await {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!(path = %dir.display(), "readdir entry error: {e}");
                    break;
                }
            };
            let entry_path = entry.path();

            let rel = entry_path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();

            // file_type() does not follow symlinks; skip symlinks to prevent escape
            let ftype = match entry.file_type().await {
                Ok(ft) => ft,
                Err(e) => {
                    tracing::warn!(path = %entry_path.display(), "file_type error: {e}");
                    continue;
                }
            };
            if ftype.is_symlink() {
                continue;
            }

            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(path = %entry_path.display(), "metadata error: {e}");
                    continue;
                }
            };

            if ftype.is_dir() {
                if recursive {
                    queue.push(entry_path);
                }
                if query
                    .prefix
                    .as_deref()
                    .map(|p| has_path_prefix(&rel, p))
                    .unwrap_or(true)
                {
                    let global_path = if rel.is_empty() {
                        scope_prefix.clone()
                    } else {
                        format!("{scope_prefix}/{rel}")
                    };
                    entries.push(Dirent {
                        path: global_path,
                        kind: DirentKind::Dir,
                        bytes: None,
                        modified_at: None,
                    });
                }
            } else if query
                .prefix
                .as_deref()
                .map(|p| has_path_prefix(&rel, p))
                .unwrap_or(true)
            {
                let modified_at = meta.modified().ok().map(DateTime::<Utc>::from);
                let global_path = if rel.is_empty() {
                    scope_prefix.clone()
                } else {
                    format!("{scope_prefix}/{rel}")
                };
                entries.push(Dirent {
                    path: global_path,
                    kind: DirentKind::File,
                    bytes: Some(meta.len()),
                    modified_at,
                });
            }
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(ListResponse { entries }))
}

/// GET /dirents/{*path}
pub async fn dirent_get_file(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    AxumPath(path_str): AxumPath<String>,
) -> ApiResult<axum::response::Response> {
    let parsed = parse_dirent_path(&path_str)?;
    if parsed.tail.as_os_str().is_empty() {
        return Err(AppError::bad_request(
            "path must point to a file, not a scope root",
        ));
    }
    enforce_scope_access(&state, &auth_user, &parsed.scope, false).await?;
    let root = scope_root(&state, &parsed.scope);
    let host_path =
        safe_join(&root, &parsed.tail.to_string_lossy()).map_err(AppError::bad_request)?;

    // symlink_metadata does not follow symlinks; reject symlinks to prevent escape
    let meta = tokio::fs::symlink_metadata(&host_path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::not_found("file not found")
        } else {
            AppError::internal(e.to_string())
        }
    })?;
    if meta.is_symlink() {
        return Err(AppError::not_found("file not found"));
    }
    if meta.is_dir() {
        return Err(AppError::bad_request("path is a directory"));
    }

    let bytes = tokio::fs::read(&host_path)
        .await
        .map_err(|e| AppError::internal(format!("failed to read file: {e}")))?;

    let content_type = mime_guess::from_path(&host_path)
        .first_or_octet_stream()
        .to_string();

    Ok(axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, content_type)
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::internal(format!("failed to build response: {e}")))?)
}

/// DELETE /dirents/{*path}
pub async fn dirent_delete(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    AxumPath(path_str): AxumPath<String>,
) -> ApiResult<StatusCode> {
    let parsed = parse_dirent_path(&path_str)?;
    if parsed.tail.as_os_str().is_empty() {
        return Err(AppError::bad_request("cannot delete a scope root"));
    }
    enforce_scope_access(&state, &auth_user, &parsed.scope, true).await?;
    let root = scope_root(&state, &parsed.scope);
    let host_path =
        safe_join(&root, &parsed.tail.to_string_lossy()).map_err(AppError::bad_request)?;

    // symlink_metadata does not follow symlinks; reject symlinks to prevent escape
    let meta = tokio::fs::symlink_metadata(&host_path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::not_found("path not found")
        } else {
            AppError::internal(e.to_string())
        }
    })?;
    if meta.is_symlink() {
        return Err(AppError::not_found("path not found"));
    }

    if meta.is_dir() {
        tokio::fs::remove_dir_all(&host_path)
            .await
            .map_err(|e| AppError::internal(format!("failed to remove directory: {e}")))?;
    } else {
        tokio::fs::remove_file(&host_path)
            .await
            .map_err(|e| AppError::internal(format!("failed to remove file: {e}")))?;
    }

    tracing::info!(path = %path_str, "dirent deleted");

    Ok(StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────────────────────────
// Batch operations (move / copy)
// ─────────────────────────────────────────────────────────────────────────────

fn validate_filename(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must not be empty".into());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("name must not contain path separators".into());
    }
    if name.contains('\0') {
        return Err("name must not contain NUL bytes".into());
    }
    if name == "." || name == ".." {
        return Err("name must not be '.' or '..'".into());
    }
    Ok(())
}

/// Empty or "/" destination resolves to the root itself.
fn resolve_destination(root: &Path, dest: &str) -> Result<PathBuf, String> {
    let trimmed = dest.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(root.to_path_buf());
    }
    safe_join(root, trimmed)
}

/// Resolve and validate a destination directory for batch ops.
///
/// Does NOT create the directory. Creation is deferred to the first
/// per-source success (via `ensure_dest_dir` inside `move_one` / `copy_one`)
/// so that a batch where every source fails (e.g. all source paths missing)
/// does not leave a stray empty directory on disk.
///
/// If the destination path already exists and is not a directory, this
/// returns a batch-wide 4xx so the outer handler can short-circuit.
async fn prepare_dest_dir(
    root: &Path,
    destination: &str,
) -> Result<PathBuf, crate::error::ApiError> {
    let dest_dir = resolve_destination(root, destination).map_err(AppError::bad_request)?;
    match tokio::fs::metadata(&dest_dir).await {
        Ok(meta) if !meta.is_dir() => Err(AppError::bad_request("destination is not a directory")),
        Ok(_) | Err(_) => Ok(dest_dir),
    }
}

/// Idempotent best-effort creation of the destination directory.
/// Called by per-source ops right before the rename/copy, so a fully-failed
/// batch (e.g. every source missing) never materialises an empty dest dir.
async fn ensure_dest_dir(dest_dir: &Path) -> Result<(), String> {
    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("create dest: {e}"))
}

/// Load and validate a source path against an already-prepared destination.
/// Used by both `move_one` and `copy_one` to avoid copy-pasting the safe-join
/// + symlink rejection + folder-into-itself check.
async fn load_source(
    root: &Path,
    src_rel: &str,
    dest_dir: &Path,
) -> Result<(PathBuf, std::fs::Metadata), String> {
    let src_host = safe_join(root, src_rel)?;
    let src_meta = tokio::fs::symlink_metadata(&src_host).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "source not found".to_string()
        } else {
            e.to_string()
        }
    })?;
    if src_meta.is_symlink() {
        return Err("source not found".into());
    }
    if src_meta.is_dir() && dest_dir.starts_with(&src_host) {
        return Err("cannot move a folder into itself or its descendants".into());
    }
    Ok((src_host, src_meta))
}

/// Build a Dirent response object from a final on-disk path.
async fn build_dirent(root: &Path, host: &Path) -> Result<Dirent, String> {
    let meta = tokio::fs::metadata(host)
        .await
        .map_err(|e| format!("metadata: {e}"))?;
    let rel = host
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let kind = if meta.is_dir() {
        DirentKind::Dir
    } else {
        DirentKind::File
    };
    let bytes = if meta.is_dir() {
        None
    } else {
        Some(meta.len())
    };
    let modified_at = meta.modified().ok().map(DateTime::<Utc>::from);
    Ok(Dirent {
        path: rel,
        kind,
        bytes,
        modified_at,
    })
}

/// "foo.pdf" → "foo copy.pdf" → "foo copy 2.pdf" → …
/// Dot-files ("foo" prefix is empty) are treated as having no extension.
///
/// Async to avoid blocking a Tokio worker on up to ~1000 sync `stat`
/// calls when `parent` happens to be a directory with a long history
/// of " copy N" siblings.
async fn find_available_name(parent: &Path, base_name: &str) -> PathBuf {
    let candidate = parent.join(base_name);
    if !tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
        return candidate;
    }
    let (stem, ext) = if base_name.starts_with('.') {
        (base_name, "")
    } else {
        match base_name.rfind('.') {
            Some(i) if i > 0 => (&base_name[..i], &base_name[i..]),
            _ => (base_name, ""),
        }
    };
    let first = parent.join(format!("{stem} copy{ext}"));
    if !tokio::fs::try_exists(&first).await.unwrap_or(false) {
        return first;
    }
    for n in 2..1000 {
        let cand = parent.join(format!("{stem} copy {n}{ext}"));
        if !tokio::fs::try_exists(&cand).await.unwrap_or(false) {
            return cand;
        }
    }
    // Fallback (effectively unreachable)
    parent.join(format!("{stem} copy {}{ext}", Uuid::new_v4().simple()))
}

/// Recursive folder copy. Symlinks are skipped to prevent escape.
///
/// Safety: callers must pass paths that were already validated via `safe_join`
/// against `root` AND confirmed non-symlink. Inside this function, all
/// further descents are by reading dir entries (no string-derived paths from
/// untrusted input), and any symlink encountered is skipped, so a malicious
/// symlink planted inside the source tree cannot escape `dst`'s subtree.
// nosemgrep: rust.actix.path-traversal.tainted-path
async fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dst).await?;
    let mut rd = tokio::fs::read_dir(src).await?;
    while let Some(entry) = rd.next_entry().await? {
        let ft = entry.file_type().await?;
        if ft.is_symlink() {
            continue;
        }
        let entry_path = entry.path();
        let target = dst.join(entry.file_name());
        if ft.is_dir() {
            Box::pin(copy_dir_recursive(&entry_path, &target)).await?;
        } else {
            tokio::fs::copy(&entry_path, &target).await?;
        }
    }
    Ok(())
}

/// Single move (rename = same destination + new_name).
async fn move_one(
    root: &Path,
    src_rel: &str,
    dest_dir: &Path,
    new_name: Option<&str>,
) -> Result<Dirent, String> {
    let (src_host, _src_meta) = load_source(root, src_rel, dest_dir).await?;

    let filename = match new_name {
        Some(n) => {
            validate_filename(n)?;
            n.to_string()
        }
        None => src_host
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "could not determine source filename".to_string())?
            .to_string(),
    };

    let new_host = dest_dir.join(&filename);

    if new_host.strip_prefix(root).is_err() {
        return Err("destination would escape root".into());
    }

    if tokio::fs::symlink_metadata(&new_host).await.is_ok() {
        return Err(format!("\"{filename}\" already exists at destination"));
    }

    ensure_dest_dir(dest_dir).await?;
    tokio::fs::rename(&src_host, &new_host)
        .await
        .map_err(|e| format!("rename failed: {e}"))?;

    build_dirent(root, &new_host).await
}

/// Single copy (folder is recursive; auto " copy" suffix on conflict).
///
/// `src_root` is the root used to resolve `src_rel`.
/// `dst_root` is the root used for the destination escape check and for
/// building the returned `Dirent` path (relative to destination scope).
async fn copy_one(
    src_root: &Path,
    src_rel: &str,
    dest_dir: &Path,
    dst_root: &Path,
) -> Result<Dirent, String> {
    let (src_host, src_meta) = load_source(src_root, src_rel, dest_dir).await?;

    let base_name = src_host
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "could not determine source filename".to_string())?
        .to_string();

    let new_host = find_available_name(dest_dir, &base_name).await;

    if new_host.strip_prefix(dst_root).is_err() {
        return Err("destination would escape root".into());
    }

    if src_meta.is_dir() {
        // `copy_dir_recursive` does its own `create_dir_all(new_host)`, which
        // also creates `dest_dir` as a side effect — no separate ensure needed.
        copy_dir_recursive(&src_host, &new_host)
            .await
            .map_err(|e| format!("copy failed: {e}"))?;
    } else {
        ensure_dest_dir(dest_dir).await?;
        tokio::fs::copy(&src_host, &new_host)
            .await
            .map_err(|e| format!("copy failed: {e}"))?;
    }

    build_dirent(dst_root, &new_host).await
}

/// PATCH /dirents
pub async fn dirent_batch_op(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Json(body): Json<DirentBatchOp>,
) -> ApiResult<Json<DirentBatchResult>> {
    // Extract all paths from the body to determine scope
    let (all_sources, destination_str) = match &body {
        DirentBatchOp::Move {
            sources,
            destination,
            ..
        } => (sources.as_slice(), destination.as_str()),
        DirentBatchOp::Copy {
            sources,
            destination,
        } => (sources.as_slice(), destination.as_str()),
    };

    if all_sources.is_empty() {
        return Err(AppError::bad_request("sources must not be empty"));
    }

    // Parse destination scope
    let dest_parsed = parse_dirent_path(destination_str)?;
    let scope = dest_parsed.scope.clone();

    // For Move: all sources must be in the same scope as destination.
    // For Copy: cross-scope is allowed within the same project.
    let is_copy = matches!(body, DirentBatchOp::Copy { .. });

    for src in all_sources {
        let src_parsed = match parse_dirent_path(src) {
            Ok(p) => p,
            Err(_) => {
                return Err(AppError::bad_request(format!(
                    "invalid source path '{src}'"
                )));
            }
        };
        if scope.matches(&src_parsed.scope) {
            // Same scope — always allowed
            continue;
        }
        if !is_copy {
            return Err(AppError::bad_request(
                "all sources and destination must be in the same scope for move operations",
            ));
        }
        // Cross-scope copy: only allowed within the same project
        if src_parsed.scope.project_id() != scope.project_id() {
            return Err(AppError::not_found("session not found")); // hide cross-project existence
        }
    }

    if is_copy {
        // Collect unique source scopes, enforce read access on each
        let mut seen: Vec<DirentScope> = Vec::new();
        for src in all_sources {
            if let Ok(p) = parse_dirent_path(src) {
                if !scope.matches(&p.scope) && !seen.iter().any(|s| s.matches(&p.scope)) {
                    enforce_scope_access(&state, &auth_user, &p.scope, false).await?;
                    seen.push(p.scope);
                }
            }
        }
    }
    // Enforce write access on destination scope
    enforce_scope_access(&state, &auth_user, &scope, true).await?;

    let root = scope_root(&state, &scope);
    let scope_prefix = scope.prefix_str();

    // Resolve destination directory (scope-relative)
    let dest_tail_str = dest_parsed.tail.to_string_lossy().to_string();
    let dest_dir = if dest_tail_str.is_empty() {
        root.clone()
    } else {
        prepare_dest_dir(&root, &dest_tail_str).await?
    };

    // Check dest_dir is not a non-directory file (only needed when non-empty tail)
    if !dest_tail_str.is_empty() {
        match tokio::fs::metadata(&dest_dir).await {
            Ok(meta) if !meta.is_dir() => {
                return Err(AppError::bad_request("destination is not a directory"));
            }
            Ok(_) | Err(_) => {}
        }
    }

    let mut succeeded: Vec<Dirent> = Vec::new();
    let mut failed: Vec<FailedFile> = Vec::new();

    match body {
        DirentBatchOp::Move {
            sources, new_name, ..
        } => {
            if new_name.is_some() && sources.len() != 1 {
                return Err(AppError::bad_request(
                    "new_name is only valid for a single source",
                ));
            }
            for src_global in &sources {
                let src_tail = match parse_dirent_path(src_global) {
                    Ok(p) => p.tail.to_string_lossy().to_string(),
                    Err(_) => {
                        failed.push(FailedFile {
                            path: src_global.clone(),
                            error: "invalid path".into(),
                        });
                        continue;
                    }
                };
                if src_tail.is_empty() {
                    failed.push(FailedFile {
                        path: src_global.clone(),
                        error: "cannot move a scope root".into(),
                    });
                    continue;
                }
                match move_one(&root, &src_tail, &dest_dir, new_name.as_deref()).await {
                    Ok(mut d) => {
                        d.path = format!("{scope_prefix}/{}", d.path);
                        succeeded.push(d);
                    }
                    Err(e) => failed.push(FailedFile {
                        path: src_global.clone(),
                        error: e,
                    }),
                }
            }
            tracing::info!(
                scope = %scope_prefix,
                count = %succeeded.len(),
                failed = %failed.len(),
                "dirents moved"
            );
        }
        DirentBatchOp::Copy { sources, .. } => {
            for src_global in &sources {
                let src_parsed = match parse_dirent_path(src_global) {
                    Ok(p) => p,
                    Err(_) => {
                        failed.push(FailedFile {
                            path: src_global.clone(),
                            error: "invalid path".into(),
                        });
                        continue;
                    }
                };
                let src_tail = src_parsed.tail.to_string_lossy().to_string();
                if src_tail.is_empty() {
                    failed.push(FailedFile {
                        path: src_global.clone(),
                        error: "cannot copy a scope root".into(),
                    });
                    continue;
                }

                // For cross-scope copy, resolve source relative to its own root
                let effective_src_root = if scope.matches(&src_parsed.scope) {
                    root.clone()
                } else {
                    scope_root(&state, &src_parsed.scope)
                };

                match copy_one(&effective_src_root, &src_tail, &dest_dir, &root).await {
                    Ok(mut d) => {
                        d.path = format!("{scope_prefix}/{}", d.path);
                        succeeded.push(d);
                    }
                    Err(e) => failed.push(FailedFile {
                        path: src_global.clone(),
                        error: e,
                    }),
                }
            }
            tracing::info!(
                scope = %scope_prefix,
                count = %succeeded.len(),
                failed = %failed.len(),
                "dirents copied"
            );
        }
    }

    Ok(Json(DirentBatchResult { succeeded, failed }))
}
