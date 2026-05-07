use axum::{Json, http::StatusCode};
use schemars::JsonSchema;
use serde::Serialize;

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

    pub fn unauthorized(msg: impl Into<String>) -> ApiError {
        (StatusCode::UNAUTHORIZED, Json(Self::new(msg)))
    }

    pub fn forbidden(msg: impl Into<String>) -> ApiError {
        (StatusCode::FORBIDDEN, Json(Self::new(msg)))
    }

    pub fn bad_request(msg: impl Into<String>) -> ApiError {
        (StatusCode::BAD_REQUEST, Json(Self::new(msg)))
    }

    pub fn conflict(msg: impl Into<String>) -> ApiError {
        (StatusCode::CONFLICT, Json(Self::new(msg)))
    }
}
