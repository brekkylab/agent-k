use schemars::JsonSchema;
use serde::Serialize;

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
    // fn not_found(msg: impl Into<String>) -> ApiErr {
    //     (StatusCode::NOT_FOUND, Json(Self { error: msg.into() }))
    // }
    // fn conflict(msg: impl Into<String>) -> ApiErr {
    //     (StatusCode::CONFLICT, Json(Self { error: msg.into() }))
    // }
    // fn bad_request(msg: impl Into<String>) -> ApiErr {
    //     (StatusCode::BAD_REQUEST, Json(Self { error: msg.into() }))
    // }
    // fn internal(msg: impl Into<String>) -> ApiErr {
    //     (
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(Self { error: msg.into() }),
    //     )
    // }
}
