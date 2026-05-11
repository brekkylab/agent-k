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
    model::{DirentEntry, DirentKind, FailedFile, ListQuery, ListResponse, UploadResponse, UploadedFile},
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
            Component::CurDir => { /* skip */ }
            Component::Normal(part) => normalized.push(part),
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err("path is empty after normalization".into());
    }

    Ok(root.join(normalized))
}

/// POST /projects/{project_id}/dirents
pub async fn upload(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    AxumPath(project_id): AxumPath<Uuid>,
    NoApi(mut multipart): NoApi<Multipart>,
) -> ApiResult<Json<UploadResponse>> {
    // 1. Membership check
    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    // 2. Resolve uploads root
    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    // 3. Create uploads root
    tokio::fs::create_dir_all(&uploads_root)
        .await
        .map_err(|e| AppError::internal(format!("failed to create uploads directory: {e}")))?;

    // 4. Per-file size limit
    let max_bytes: usize = std::env::var("AGENT_K_MAX_UPLOAD_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50 * 1024 * 1024);

    let mut succeeded: Vec<UploadedFile> = Vec::new();
    let mut failed: Vec<FailedFile> = Vec::new();

    // 5. Process each multipart field
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?
    {
        // Get filename
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

        // Validate path
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

        // Read bytes
        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::bad_request(format!("multipart error: {e}")))?;

        // Check size limit
        if data.len() > max_bytes {
            failed.push(FailedFile {
                path: filename,
                error: format!(
                    "file exceeds maximum size ({} bytes > {} bytes)",
                    data.len(),
                    max_bytes
                ),
            });
            continue;
        }

        // Create parent dirs (Fix C1 + I1: use uploads_root-based tmp_path)
        tokio::fs::create_dir_all(host_path.parent().unwrap())
            .await
            .map_err(|e| AppError::internal(format!("failed to create dirs: {e}")))?;

        // Atomic write: write to temp file then rename
        // Fix I1: temp file lives in uploads_root to avoid with_extension side-effect
        let tmp_path = uploads_root.join(format!(".tmp.{}", Uuid::new_v4().simple()));
        tokio::fs::write(&tmp_path, &data)
            .await
            .map_err(|e| AppError::internal(format!("failed to write temp file: {e}")))?;
        // Fix C1: clean up temp file if rename fails
        if let Err(e) = tokio::fs::rename(&tmp_path, &host_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(AppError::internal(format!("failed to finalize file: {e}")));
        }

        succeeded.push(UploadedFile {
            path: filename,
            bytes: data.len() as u64,
        });
    }

    // Fix m1: trace successful uploads
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
    // 1. Membership check
    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    // 2. Resolve uploads root
    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    // 3. Return empty response if directory doesn't exist (Fix I2: async metadata check)
    match tokio::fs::metadata(&uploads_root).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(ListResponse { project_id, entries: vec![] }));
        }
        Err(e) => return Err(AppError::internal(e.to_string())),
        Ok(_) => {}
    }

    // 4. BFS directory walk
    let recursive = query.recursive.unwrap_or(true);
    let mut entries: Vec<DirentEntry> = Vec::new();
    let mut queue: Vec<PathBuf> = vec![uploads_root.clone()];

    while let Some(dir) = queue.pop() {
        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let entry_path = entry.path();

            // Compute relative path
            let rel = entry_path
                .strip_prefix(&uploads_root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();

            // Fix I3: check file type without following symlinks; skip symlinks entirely
            let ftype = match entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ftype.is_symlink() {
                continue; // never follow symlinks
            }

            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Fix m4: separate recurse decision from prefix filter
            if ftype.is_dir() {
                // Always recurse when recursive=true (needed to find matching children below)
                if recursive {
                    queue.push(entry_path);
                }
                // Only include this dir in entries if its path matches the prefix
                if query.prefix.as_deref().map(|p| rel.starts_with(p)).unwrap_or(true) {
                    entries.push(DirentEntry {
                        path: rel,
                        kind: DirentKind::Dir,
                        bytes: None,
                        modified_at: None,
                    });
                }
            } else {
                // Files only included if they match the prefix
                if query.prefix.as_deref().map(|p| rel.starts_with(p)).unwrap_or(true) {
                    let modified_at = meta
                        .modified()
                        .ok()
                        .map(|st| DateTime::<Utc>::from(st));
                    entries.push(DirentEntry {
                        path: rel,
                        kind: DirentKind::File,
                        bytes: Some(meta.len()),
                        modified_at,
                    });
                }
            }
        }
    }

    // 5. Sort by path for deterministic output
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
    // 1. Membership check
    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    // 2. Resolve uploads root
    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    // 3. Validate path
    let host_path =
        safe_join(&uploads_root, &path_str).map_err(|e| AppError::bad_request(e))?;

    // 4. Check metadata
    let meta = tokio::fs::metadata(&host_path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::not_found("file not found")
        } else {
            AppError::internal(e.to_string())
        }
    })?;

    // 5. Reject directories
    if meta.is_dir() {
        return Err(AppError::bad_request("path is a directory"));
    }

    // 6. Read file
    let bytes = tokio::fs::read(&host_path)
        .await
        .map_err(|e| AppError::internal(format!("failed to read file: {e}")))?;

    // 7. Detect content type
    let content_type = mime_guess::from_path(&host_path)
        .first_or_octet_stream()
        .to_string();

    // 8. Build response (Fix I5: propagate builder error instead of unwrap)
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
    // 1. Membership check
    let in_project = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !in_project {
        return Err(AppError::forbidden("not a member of this project"));
    }

    // 2. Resolve uploads root
    let uploads_root = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");

    // 3. Validate path
    let host_path =
        safe_join(&uploads_root, &path_str).map_err(|e| AppError::bad_request(e))?;

    // 4. Check metadata
    let meta = tokio::fs::metadata(&host_path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::not_found("path not found")
        } else {
            AppError::internal(e.to_string())
        }
    })?;

    // 5. Remove dir or file
    if meta.is_dir() {
        tokio::fs::remove_dir_all(&host_path)
            .await
            .map_err(|e| AppError::internal(format!("failed to remove directory: {e}")))?;
    } else {
        tokio::fs::remove_file(&host_path)
            .await
            .map_err(|e| AppError::internal(format!("failed to remove file: {e}")))?;
    }

    // Fix m1: trace deletion
    tracing::info!(project = %project_id, path = %path_str, "dirent deleted");

    Ok(StatusCode::NO_CONTENT)
}
