//! Typed error for provider [`Resource`](crate::vfs::Resource) operations.
//!
//! The trait returns [`VfsError`] rather than `anyhow::Error` so callers can
//! classify failures precisely — most importantly [`NotFound`](VfsError::NotFound),
//! which the WebDAV layer must translate into a 404 rather than a 500. Backend
//! failures (network, serialization, upstream API errors) collapse into
//! [`Backend`](VfsError::Backend), preserving the original error for logging.

/// An error from a provider [`Resource`](crate::vfs::Resource) operation.
#[derive(Debug)]
pub enum VfsError {
    /// The requested path does not exist.
    NotFound,
    /// The operation isn't supported by this provider (e.g. a write to a
    /// read-only mount, or an optional op a provider doesn't implement).
    Unsupported,
    /// Any other backend failure (network, serialization, upstream API error).
    Backend(anyhow::Error),
}

pub type VfsResult<T> = Result<T, VfsError>;

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VfsError::NotFound => write!(f, "not found"),
            VfsError::Unsupported => write!(f, "operation not supported"),
            VfsError::Backend(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for VfsError {}

impl From<anyhow::Error> for VfsError {
    fn from(e: anyhow::Error) -> Self {
        VfsError::Backend(e)
    }
}

impl From<object_store::Error> for VfsError {
    fn from(e: object_store::Error) -> Self {
        // Surface a missing object as NotFound; everything else is a backend
        // failure carrying the original error.
        match e {
            object_store::Error::NotFound { .. } => VfsError::NotFound,
            other => VfsError::Backend(other.into()),
        }
    }
}

impl From<serde_json::Error> for VfsError {
    fn from(e: serde_json::Error) -> Self {
        VfsError::Backend(e.into())
    }
}

impl From<reqwest::Error> for VfsError {
    fn from(e: reqwest::Error) -> Self {
        VfsError::Backend(e.into())
    }
}

impl From<std::io::Error> for VfsError {
    fn from(e: std::io::Error) -> Self {
        VfsError::Backend(e.into())
    }
}
