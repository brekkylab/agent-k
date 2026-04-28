use schemars::JsonSchema;
use serde::Serialize;

use axum::{Json, http::StatusCode};

pub type ApiError = (StatusCode, Json<AppError>);
pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, JsonSchema, Serialize)]
pub struct AppError {
    error: String,
}

impl AppError {
    pub fn new(errstr: impl Into<String>) -> Self {
        Self {
            error: errstr.into(),
        }
    }

    pub fn internal(msg: impl Into<String>) -> ApiError {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(Self::new(msg)))
    }

    pub fn not_found(msg: impl Into<String>) -> ApiError {
        (StatusCode::NOT_FOUND, Json(Self::new(msg)))
    }
}
