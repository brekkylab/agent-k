//! Typed error for provider [`Resource`](crate::vfs::Resource) operations.
//!
//! The trait returns [`ResourceError`] rather than `anyhow::Error` so callers can
//! classify failures precisely — most importantly [`NotFound`](ResourceError::NotFound),
//! which the WebDAV layer must translate into a 404 rather than a 500. Backend
//! failures (network, serialization, upstream API errors) collapse into
//! [`Backend`](ResourceError::Backend), preserving the original error for logging.

/// An error from a provider [`Resource`](crate::vfs::Resource) operation.
#[derive(Debug)]
pub enum ResourceError {
    /// The requested path does not exist.
    NotFound,
    /// The operation isn't supported by this provider (e.g. a write to a
    /// read-only mount, or an optional op a provider doesn't implement).
    Unsupported,
    /// Any other backend failure (network, serialization, upstream API error).
    Backend(anyhow::Error),
}

pub type ResourceResult<T> = Result<T, ResourceError>;

impl std::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceError::NotFound => write!(f, "not found"),
            ResourceError::Unsupported => write!(f, "operation not supported"),
            ResourceError::Backend(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ResourceError {}

impl From<anyhow::Error> for ResourceError {
    fn from(e: anyhow::Error) -> Self {
        ResourceError::Backend(e)
    }
}

impl From<object_store::Error> for ResourceError {
    fn from(e: object_store::Error) -> Self {
        // Surface a missing object as NotFound; everything else is a backend
        // failure carrying the original error.
        match e {
            object_store::Error::NotFound { .. } => ResourceError::NotFound,
            other => ResourceError::Backend(other.into()),
        }
    }
}

impl From<serde_json::Error> for ResourceError {
    fn from(e: serde_json::Error) -> Self {
        ResourceError::Backend(e.into())
    }
}

impl From<reqwest::Error> for ResourceError {
    fn from(e: reqwest::Error) -> Self {
        ResourceError::Backend(e.into())
    }
}

impl From<std::io::Error> for ResourceError {
    fn from(e: std::io::Error) -> Self {
        ResourceError::Backend(e.into())
    }
}
