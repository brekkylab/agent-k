//! A mount that exists but cannot serve, because the deployment is missing something
//! the provider needs.
//!
//! The alternative shapes are both worse. Failing [`build_mounts`] takes the whole
//! filesystem down, `/files` included, in every workspace that happens to hold such a
//! mount. Leaving the mount out looks like success: the prefix is unroutable, so every
//! op answers `NotFound`, and callers that treat a missing path as "nothing to do here"
//! act on that. The knowledge index is one, and its reaction is to purge every document
//! it had ingested from the source, which turns a recoverable misconfiguration into
//! deleted data.
//!
//! So the mount stays in the table and answers [`ResourceError::Backend`] instead. That
//! is the same shape a provider produces when its credential is rejected, which callers
//! already have to survive: a failed pass is left to be retried rather than treated as
//! an answer. The mount also keeps its name, so anything listing what is connected
//! still says so rather than quietly dropping it.
//!
//! [`build_mounts`]: crate::vfs::mount::build_mounts

use async_trait::async_trait;

use crate::vfs::{
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileStat, Resource},
};

/// A provider mount whose prerequisites the deployment did not supply. Every op fails
/// with `why`.
pub struct UnavailableResource {
    why: String,
}

impl UnavailableResource {
    /// `why` is returned from every operation, so it should say what is missing and
    /// where it is configured.
    pub fn new(why: impl Into<String>) -> Self {
        Self { why: why.into() }
    }

    /// Deliberately not `NotFound`: the path may well exist upstream, and a caller that
    /// distinguishes the two must be able to tell that this is a failure to reach it.
    fn fail<T>(&self) -> ResourceResult<T> {
        Err(ResourceError::Backend(anyhow::anyhow!("{}", self.why)))
    }
}

#[async_trait]
impl Resource for UnavailableResource {
    async fn read_bytes(
        &self,
        _path: &MountPath,
        _range: Option<std::ops::Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        self.fail()
    }

    async fn read_bytes_pinned(
        &self,
        _path: &MountPath,
        _range: Option<std::ops::Range<u64>>,
        _stat: &FileStat,
    ) -> ResourceResult<Vec<u8>> {
        self.fail()
    }

    async fn write_bytes(&self, _path: &MountPath, _data: Vec<u8>) -> ResourceResult<()> {
        self.fail()
    }

    async fn readdir(&self, _path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        self.fail()
    }

    async fn stat(&self, _path: &MountPath) -> ResourceResult<FileStat> {
        self.fail()
    }
}
