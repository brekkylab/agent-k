use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::models::{
    CreateSpeedwagonRequest, ErrorResponse, Speedwagon, SpeedwagonIndexStatus,
    UpdateSpeedwagonRequest,
};
use crate::repository::RepositoryError;
use crate::state::AppState;

#[derive(Debug, thiserror::Error)]
pub enum SpeedwagonError {
    #[error("speedwagon not found")]
    NotFound,
    #[error("indexing already in progress")]
    AlreadyIndexing,
    #[error("speedwagon has no sources")]
    NoSources,
    #[error("name must not be empty")]
    EmptyName,
    #[error("lm and provider_profile_id must be both set or both unset")]
    InconsistentOverride,
    #[error(transparent)]
    Repository(#[from] RepositoryError),
}

impl IntoResponse for SpeedwagonError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::AlreadyIndexing => StatusCode::CONFLICT,
            Self::NoSources => StatusCode::UNPROCESSABLE_ENTITY,
            Self::EmptyName | Self::InconsistentOverride => StatusCode::BAD_REQUEST,
            Self::Repository(e) => {
                tracing::error!("repository error: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let error_msg = if matches!(self, Self::Repository(_)) {
            "internal server error".to_string()
        } else {
            self.to_string()
        };
        (status, Json(ErrorResponse { error: error_msg })).into_response()
    }
}

/// Validate that `lm` and `provider_profile_id` are both-Some or both-None.
fn validate_override(
    lm: &Option<String>,
    provider_profile_id: Option<Uuid>,
) -> Result<Option<Uuid>, SpeedwagonError> {
    if lm.is_some() != provider_profile_id.is_some() {
        return Err(SpeedwagonError::InconsistentOverride);
    }
    Ok(provider_profile_id)
}

/// Create a new speedwagon with the given sources.
pub async fn create_speedwagon(
    state: &AppState,
    req: CreateSpeedwagonRequest,
) -> Result<Speedwagon, SpeedwagonError> {
    let CreateSpeedwagonRequest {
        name,
        description,
        instruction,
        lm,
        provider_profile_id,
        source_ids,
    } = req;

    if name.trim().is_empty() {
        return Err(SpeedwagonError::EmptyName);
    }

    let provider_profile_id = validate_override(&lm, provider_profile_id)?;

    let sw = state
        .repository
        .create_speedwagon(
            name,
            description,
            instruction,
            lm,
            provider_profile_id,
            source_ids,
        )
        .await?;

    Ok(sw)
}

/// Update a speedwagon's name, description, instruction, lm, and sources.
pub async fn update_speedwagon(
    state: &AppState,
    id: Uuid,
    req: UpdateSpeedwagonRequest,
) -> Result<Speedwagon, SpeedwagonError> {
    let UpdateSpeedwagonRequest {
        name,
        description,
        instruction,
        lm,
        provider_profile_id,
        source_ids,
    } = req;

    let provider_profile_id = validate_override(&lm, provider_profile_id)?;

    let sw = state
        .repository
        .update_speedwagon(
            id,
            name,
            description,
            instruction,
            lm,
            provider_profile_id,
            source_ids,
        )
        .await?
        .ok_or(SpeedwagonError::NotFound)?;

    // Invalidate runtime cache for sessions using this speedwagon
    if let Ok(session_ids) = state.repository.get_sessions_by_speedwagon_id(id).await {
        for session_id in session_ids {
            state.invalidate_session_runtime(session_id);
        }
    }

    Ok(sw)
}

/// Delete a speedwagon from DB and clean up its data directory on disk.
pub async fn delete_speedwagon(state: &AppState, id: Uuid) -> Result<(), SpeedwagonError> {
    // Get speedwagon first to find data directory
    let sw = state
        .repository
        .get_speedwagon(id)
        .await?
        .ok_or(SpeedwagonError::NotFound)?;

    // Invalidate runtime cache BEFORE delete (ON DELETE CASCADE removes session_speedwagons rows)
    if let Ok(session_ids) = state.repository.get_sessions_by_speedwagon_id(id).await {
        for session_id in session_ids {
            state.invalidate_session_runtime(session_id);
        }
    }

    let deleted = state.repository.delete_speedwagon(id).await?;
    if !deleted {
        return Err(SpeedwagonError::NotFound);
    }

    // Clean up disk data
    let dir = state.speedwagon_data_dir.join(sw.id.to_string());
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::error!("failed to delete speedwagon dir {}: {e}", dir.display());
        }
    }

    Ok(())
}

/// Validates and sets status to Indexing. Returns the speedwagon for the caller to spawn the indexing task.
pub async fn start_indexing(state: &AppState, id: Uuid) -> Result<Speedwagon, SpeedwagonError> {
    let sw = state
        .repository
        .get_speedwagon(id)
        .await?
        .ok_or(SpeedwagonError::NotFound)?;

    if sw.source_ids.is_empty() {
        return Err(SpeedwagonError::NoSources);
    }

    if sw.index_status == SpeedwagonIndexStatus::Indexing {
        return Err(SpeedwagonError::AlreadyIndexing);
    }

    let now = chrono::Utc::now();
    state
        .repository
        .update_speedwagon_index_status(
            id,
            SpeedwagonIndexStatus::Indexing,
            None,
            None,
            None,
            Some(now),
            None,
        )
        .await?;

    Ok(sw)
}
