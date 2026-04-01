use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError};
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
    #[error(transparent)]
    Repository(#[from] RepositoryError),
}

impl ResponseError for SpeedwagonError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::AlreadyIndexing => StatusCode::CONFLICT,
            Self::NoSources => StatusCode::UNPROCESSABLE_ENTITY,
            Self::EmptyName => StatusCode::BAD_REQUEST,
            Self::Repository(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let Self::Repository(e) = self {
            eprintln!("repository error: {e}");
            return HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR)
                .json(ErrorResponse { error: "internal server error".to_string() });
        }
        HttpResponse::build(self.status_code()).json(ErrorResponse {
            error: self.to_string(),
        })
    }
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
        source_ids,
    } = req;

    if name.trim().is_empty() {
        return Err(SpeedwagonError::EmptyName);
    }

    let sw = state
        .repository
        .create_speedwagon(name, description, instruction, lm, source_ids)
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
        source_ids,
    } = req;

    let sw = state
        .repository
        .update_speedwagon(id, name, description, instruction, lm, source_ids)
        .await?
        .ok_or(SpeedwagonError::NotFound)?;

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

    let deleted = state.repository.delete_speedwagon(id).await?;
    if !deleted {
        return Err(SpeedwagonError::NotFound);
    }

    // Clean up disk data
    let dir = state.speedwagon_data_dir.join(sw.id.to_string());
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            eprintln!("failed to delete speedwagon dir {}: {e}", dir.display());
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
