//! [`ForwardFs`] — the filesystem surface the host [`ForwardServer`](super::ForwardServer)
//! server exposes to the in-guest FUSE forwarder.
//!
//! Implemented by `WorkspaceFs` (in [`crate::state`]) — the unified workspace
//! tree: the workspace's local files under `files/` plus the provider mounts as
//! sibling subdirectories, i.e. what the browser sees over WebDAV.
//!
//! Reads are required; the mutating ops default to a read-only rejection so a
//! read-only frontend need not implement them. Errors are `anyhow` because this
//! is the HTTP forward layer, not the typed provider core.

use std::time::SystemTime;

use async_trait::async_trait;

/// A directory entry as the forward server reports it.
pub struct FwdEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Modification time, seconds since the Unix epoch, if the source reports one.
    pub mtime: Option<u64>,
}

/// A stat result. `exists == false` collapses every "no such node" outcome
/// (missing path, unroutable prefix, backend stat error) into a single answer
/// the FUSE forwarder turns into `ENOENT`.
pub struct FwdStat {
    pub exists: bool,
    pub is_dir: bool,
    pub size: u64,
    /// Times as seconds since the Unix epoch, if the source reports them.
    /// `atime`/`ctime` are `None` for backends that only track modification
    /// time (e.g. S3); the forwarder falls back to `mtime` when they are absent.
    pub mtime: Option<u64>,
    pub atime: Option<u64>,
    pub ctime: Option<u64>,
}

impl FwdStat {
    /// The canonical "no such node" answer.
    pub fn missing() -> Self {
        Self {
            exists: false,
            is_dir: false,
            size: 0,
            mtime: None,
            atime: None,
            ctime: None,
        }
    }
}

/// The filesystem the [`ForwardServer`](super::ForwardServer) server serves over HTTP.
/// Implemented by `WorkspaceFs` (the unified workspace tree).
#[async_trait]
pub trait ForwardFs: Send + Sync + 'static {
    async fn readdir(&self, path: &str) -> anyhow::Result<Vec<FwdEntry>>;

    async fn stat(&self, path: &str) -> anyhow::Result<FwdStat>;

    async fn read(
        &self,
        path: &str,
        offset: Option<u64>,
        size: Option<u64>,
    ) -> anyhow::Result<Vec<u8>>;

    /// Write `data` to `path`, returning the JSON body to hand back. Defaults to
    /// a read-only rejection.
    async fn write(&self, _path: &str, _data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("read-only filesystem")
    }

    async fn unlink(&self, _path: &str) -> anyhow::Result<()> {
        anyhow::bail!("read-only filesystem")
    }

    async fn mkdir(&self, _path: &str) -> anyhow::Result<()> {
        anyhow::bail!("read-only filesystem")
    }

    async fn rmdir(&self, _path: &str) -> anyhow::Result<()> {
        anyhow::bail!("read-only filesystem")
    }

    async fn rename(&self, _from: &str, _to: &str) -> anyhow::Result<()> {
        anyhow::bail!("read-only filesystem")
    }
}

/// Seconds-since-epoch of a [`SystemTime`], for the wire representation.
pub(crate) fn secs_since_epoch(t: SystemTime) -> Option<u64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
