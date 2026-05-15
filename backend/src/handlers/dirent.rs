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
    error::{ApiResult, AppError},
    model::{
        Dirent, DirentKind, FailedFile, ListQuery, ListResponse, UploadResponse, UploadedFile,
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
) -> ApiResult<Json<UploadResponse>> {
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
    let mut succeeded: Vec<UploadedFile> = Vec::new();
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

        succeeded.push(UploadedFile {
            path: filename,
            bytes: byte_count,
        });
    }

    tracing::info!(project = %project_id, count = %succeeded.len(), "dirents uploaded");

    Ok(Json(UploadResponse {
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
