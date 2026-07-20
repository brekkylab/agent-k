use std::sync::Arc;

use axum::{Json, extract::State};

use crate::{model::ModelCatalogResponse, state::AppState};

use super::error::ApiError;

/// GET /models — the model catalog grouped by tier, annotated with live
/// provider availability, plus each agent surface's recommendation chain.
pub(super) async fn list_models(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<ModelCatalogResponse>, ApiError> {
    Ok(Json(crate::model::catalog_response()))
}
