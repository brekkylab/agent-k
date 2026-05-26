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
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiError, ApiResult, AppError},
    model::{
        Dirent, DirentBatchOp, DirentBatchResult, DirentKind, FailedFile, ListQuery, ListResponse,
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

/// POST /projects/{project_id}/dirents
pub async fn upload(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    AxumPath(project_id): AxumPath<Uuid>,
    NoApi(mut multipart): NoApi<Multipart>,
) -> ApiResult<Json<DirentBatchResult>> {

    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    tokio::fs::create_dir_all(&uploads_root)
        .await
        .map_err(|e| AppError::internal(format!("failed to create uploads directory: {e}")))?;

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

        let host_path = match safe_join(&uploads_root, &filename) {
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
        let tmp_path = uploads_root.join(format!(".tmp.{}", Uuid::new_v4().simple()));
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
            path: filename,
            kind: DirentKind::File,
            bytes: Some(byte_count),
            modified_at,
        });
    }

    tracing::info!(project = %project_id, count = %succeeded.len(), "dirents uploaded");

    Ok(Json(DirentBatchResult {
        project_id,
        succeeded,
        failed,
    }))
}

/// GET /projects/{project_id}/dirents
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    AxumPath(project_id): AxumPath<Uuid>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<ListResponse>> {

    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    match tokio::fs::metadata(&uploads_root).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(ListResponse {
                project_id,
                entries: vec![],
            }));
        }
        Err(e) => return Err(AppError::internal(e.to_string())),
        Ok(_) => {}
    }

    let recursive = query.recursive.unwrap_or(true);
    let mut entries: Vec<Dirent> = Vec::new();
    let mut queue: Vec<PathBuf> = vec![uploads_root.clone()];

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
                .strip_prefix(&uploads_root)
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
                    entries.push(Dirent {
                        path: rel,
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
                entries.push(Dirent {
                    path: rel,
                    kind: DirentKind::File,
                    bytes: Some(meta.len()),
                    modified_at,
                });
            }
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(ListResponse {
        project_id,
        entries,
    }))
}

/// GET /projects/{project_id}/dirents/{*path}
pub async fn get_file(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    AxumPath((project_id, path_str)): AxumPath<(Uuid, String)>,
) -> ApiResult<axum::response::Response> {

    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    let host_path = safe_join(&uploads_root, &path_str).map_err(|e| AppError::bad_request(e))?;

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

/// DELETE /projects/{project_id}/dirents/{*path}
pub async fn delete_path(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    AxumPath((project_id, path_str)): AxumPath<(Uuid, String)>,
) -> ApiResult<StatusCode> {

    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    let host_path = safe_join(&uploads_root, &path_str).map_err(|e| AppError::bad_request(e))?;

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

    tracing::info!(project = %project_id, path = %path_str, "dirent deleted");

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

/// Empty or "/" destination resolves to the uploads_root itself.
fn resolve_destination(uploads_root: &Path, dest: &str) -> Result<PathBuf, String> {
    let trimmed = dest.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(uploads_root.to_path_buf());
    }
    safe_join(uploads_root, trimmed)
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
async fn prepare_dest_dir(uploads_root: &Path, destination: &str) -> Result<PathBuf, ApiError> {
    let dest_dir = resolve_destination(uploads_root, destination).map_err(AppError::bad_request)?;
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
    uploads_root: &Path,
    src_rel: &str,
    dest_dir: &Path,
) -> Result<(PathBuf, std::fs::Metadata), String> {
    let src_host = safe_join(uploads_root, src_rel)?;
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
async fn build_dirent(uploads_root: &Path, host: &Path) -> Result<Dirent, String> {
    let meta = tokio::fs::metadata(host)
        .await
        .map_err(|e| format!("metadata: {e}"))?;
    let rel = host
        .strip_prefix(uploads_root)
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
/// against `uploads_root` AND confirmed non-symlink. Inside this function, all
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
    uploads_root: &Path,
    src_rel: &str,
    dest_dir: &Path,
    new_name: Option<&str>,
) -> Result<Dirent, String> {
    let (src_host, _src_meta) = load_source(uploads_root, src_rel, dest_dir).await?;

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

    if new_host.strip_prefix(uploads_root).is_err() {
        return Err("destination would escape uploads root".into());
    }

    if tokio::fs::symlink_metadata(&new_host).await.is_ok() {
        return Err(format!("\"{filename}\" already exists at destination"));
    }

    ensure_dest_dir(dest_dir).await?;
    tokio::fs::rename(&src_host, &new_host)
        .await
        .map_err(|e| format!("rename failed: {e}"))?;

    build_dirent(uploads_root, &new_host).await
}

/// Single copy (folder is recursive; auto " copy" suffix on conflict).
async fn copy_one(uploads_root: &Path, src_rel: &str, dest_dir: &Path) -> Result<Dirent, String> {
    let (src_host, src_meta) = load_source(uploads_root, src_rel, dest_dir).await?;

    let base_name = src_host
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "could not determine source filename".to_string())?
        .to_string();

    let new_host = find_available_name(dest_dir, &base_name).await;

    if new_host.strip_prefix(uploads_root).is_err() {
        return Err("destination would escape uploads root".into());
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

    build_dirent(uploads_root, &new_host).await
}

/// PATCH /projects/{project_id}/dirents — batch move/copy operations.
pub async fn batch_op(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    AxumPath(project_id): AxumPath<Uuid>,
    Json(body): Json<DirentBatchOp>,
) -> ApiResult<Json<DirentBatchResult>> {

    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    let mut succeeded: Vec<Dirent> = Vec::new();
    let mut failed: Vec<FailedFile> = Vec::new();

    match body {
        DirentBatchOp::Move {
            sources,
            destination,
            new_name,
        } => {
            if sources.is_empty() {
                return Err(AppError::bad_request("sources must not be empty"));
            }
            if new_name.is_some() && sources.len() != 1 {
                return Err(AppError::bad_request(
                    "new_name is only valid for a single source",
                ));
            }
            let dest_dir = prepare_dest_dir(&uploads_root, &destination).await?;
            for src in sources {
                match move_one(&uploads_root, &src, &dest_dir, new_name.as_deref()).await {
                    Ok(d) => succeeded.push(d),
                    Err(e) => failed.push(FailedFile {
                        path: src,
                        error: e,
                    }),
                }
            }
            tracing::info!(
                project = %project_id,
                count = %succeeded.len(),
                failed = %failed.len(),
                "dirents moved"
            );
        }
        DirentBatchOp::Copy {
            sources,
            destination,
        } => {
            if sources.is_empty() {
                return Err(AppError::bad_request("sources must not be empty"));
            }
            let dest_dir = prepare_dest_dir(&uploads_root, &destination).await?;
            for src in sources {
                match copy_one(&uploads_root, &src, &dest_dir).await {
                    Ok(d) => succeeded.push(d),
                    Err(e) => failed.push(FailedFile {
                        path: src,
                        error: e,
                    }),
                }
            }
            tracing::info!(
                project = %project_id,
                count = %succeeded.len(),
                failed = %failed.len(),
                "dirents copied"
            );
        }
    }

    Ok(Json(DirentBatchResult {
        project_id,
        succeeded,
        failed,
    }))
}
