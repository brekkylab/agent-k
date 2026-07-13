use axum::{Json, http::StatusCode};
use schemars::JsonSchema;
use serde::Serialize;

use crate::state::StateError;

pub type ApiError = (StatusCode, Json<ErrorBody>);

#[derive(Debug, Serialize, JsonSchema)]
pub struct ErrorBody {
    pub error: String,
}

impl ErrorBody {
    fn new(msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
        }
    }
}

pub fn err(status: StatusCode, msg: impl Into<String>) -> ApiError {
    (status, Json(ErrorBody::new(msg)))
}

impl From<StateError> for ApiError {
    fn from(e: StateError) -> Self {
        match e {
            StateError::NotFound => err(StatusCode::NOT_FOUND, "not found"),
            StateError::InvalidData(msg) => err(StatusCode::BAD_REQUEST, msg),
            StateError::UniqueViolation(col) => {
                err(StatusCode::CONFLICT, format!("conflict on {col}"))
            }
            StateError::AlreadyRunning(_) => {
                err(StatusCode::CONFLICT, "session is already running")
            }
            other => {
                tracing::error!("state error: {other}");
                err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
            }
        }
    }
}
