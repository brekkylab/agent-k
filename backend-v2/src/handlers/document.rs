use std::sync::Arc;

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use speedwagon::FileType;
use uuid::Uuid;

use crate::{
    error::{ApiResult, AppError},
    model::{
        BatchIngestResponse, BatchPurgeResponse, BulkPurgeRequest, DocumentResponse, FailedItem,
    },
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct ListDocumentsQuery {
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub page_size: Option<u32>,
}

pub async fn list_documents(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListDocumentsQuery>,
) -> ApiResult<Json<Vec<DocumentResponse>>> {
    let page = query.page.unwrap_or(0);
    let page_size = query.page_size.unwrap_or(50);

    let store = state.store.read().await;
    let docs = store
        .list(false, page, page_size)
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(docs.into_iter().map(DocumentResponse::from).collect()))
}

pub async fn get_document(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DocumentResponse>> {
    let store = state.store.read().await;
    match store.get(id) {
        Some(doc) => Ok(Json(DocumentResponse::from(doc))),
        None => Err(AppError::not_found("document not found")),
    }
}

fn parse_filetype(filename: &str) -> Result<FileType, String> {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" => Ok(FileType::PDF),
        "md" | "markdown" | "txt" => Ok(FileType::MD),
        _ => Err(format!(
            "unsupported file type '.{ext}' — supported: pdf, md, txt"
        )),
    }
}

pub async fn ingest_document(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<BatchIngestResponse>)> {
    let mut valid_items: Vec<(Vec<u8>, FileType)> = Vec::new();
    let mut filenames: Vec<String> = Vec::new();
    let mut failed: Vec<FailedItem> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or("upload").to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| AppError::internal(format!("failed to read upload: {e}")))?;

        match parse_filetype(&filename) {
            Ok(filetype) => {
                valid_items.push((bytes.to_vec(), filetype));
                filenames.push(filename);
            }
            Err(e) => {
                failed.push(FailedItem {
                    name: filename,
                    error: e,
                });
            }
        }
    }

    if valid_items.is_empty() && failed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(AppError::new("missing 'file' field in multipart body")),
        ));
    }

    let mut store = state.store.write().await;
    let result = store
        .ingest_many(valid_items)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let docs = store
        .get_many(&result.succeeded)
        .map_err(|e| AppError::internal(e.to_string()))?;
    drop(store);

    for f in result.failed {
        let name = filenames
            .get(f.index)
            .cloned()
            .unwrap_or_else(|| format!("file[{}]", f.index));
        failed.push(FailedItem {
            name,
            error: f.error,
        });
    }

    for doc in &docs {
        tracing::info!(id = %doc.id, title = %doc.title, "document ingested");
    }

    let succeeded: Vec<DocumentResponse> = docs.into_iter().map(DocumentResponse::from).collect();
    let status = if failed.is_empty() {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Ok((status, Json(BatchIngestResponse { succeeded, failed })))
}

pub async fn purge_document(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mut store = state.store.write().await;
    match store
        .purge(id)
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        Some(doc) => {
            tracing::info!(%id, title = %doc.title, "document purged");
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(AppError::not_found("document not found")),
    }
}

pub async fn purge_documents(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BulkPurgeRequest>,
) -> ApiResult<(StatusCode, Json<BatchPurgeResponse>)> {
    if payload.ids.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(AppError::new("ids must not be empty")),
        ));
    }

    let mut store = state.store.write().await;
    let result = store.purge_many(payload.ids);
    drop(store);

    let purged: Vec<String> = result.purged.iter().map(|id| id.to_string()).collect();
    let failed: Vec<FailedItem> = result
        .failed
        .into_iter()
        .map(|f| FailedItem {
            name: f.id.to_string(),
            error: f.error,
        })
        .collect();

    for id in &purged {
        tracing::info!(%id, "document purged");
    }

    Ok((StatusCode::OK, Json(BatchPurgeResponse { purged, failed })))
}
