//! [`ForwardFs`] — the filesystem surface the host [`VfsForward`](super::VfsForward)
//! server exposes to the in-guest FUSE forwarder.
//!
//! Two frontends implement it:
//!
//! * [`Vfs`] (here) — provider mounts only (`/notion`, `/s3-…`). This is the
//!   original, provider-only view the forwarder started with.
//! * `WorkspaceFs` (in [`crate::state`]) — the **unified** workspace tree: the
//!   workspace's local files at the root plus the provider mounts as virtual
//!   subdirectories, i.e. exactly what the browser sees over WebDAV.
//!
//! Reads are required; the mutating ops default to a read-only rejection so a
//! read-only frontend need not implement them. Errors are `anyhow` because this
//! is the HTTP forward layer, not the typed provider core.

use std::time::SystemTime;

use async_trait::async_trait;

use crate::vfs::{Vfs, resource::FileKind};

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
    pub mtime: Option<u64>,
}

impl FwdStat {
    /// The canonical "no such node" answer.
    pub fn missing() -> Self {
        Self {
            exists: false,
            is_dir: false,
            size: 0,
            mtime: None,
        }
    }
}

/// The filesystem the [`VfsForward`](super::VfsForward) server serves over HTTP.
/// Implemented by [`Vfs`] (provider-only) and `WorkspaceFs` (unified).
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

/// Provider-only view: virtual paths route to their mount by longest prefix, and
/// the root lists the mount names. This preserves the forward server's original
/// behaviour verbatim, including the `.cmd/<op>` domain-command write path.
#[async_trait]
impl ForwardFs for Vfs {
    async fn readdir(&self, path: &str) -> anyhow::Result<Vec<FwdEntry>> {
        if path == "/" {
            return Ok(self
                .mount_names()
                .into_iter()
                .map(|name| FwdEntry {
                    name,
                    is_dir: true,
                    size: 0,
                    mtime: None,
                })
                .collect());
        }
        let (res, vp) = self
            .route(path)
            .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
        Ok(res
            .readdir(&vp)
            .await?
            .into_iter()
            .map(|e| FwdEntry {
                name: e.name,
                is_dir: matches!(e.kind, FileKind::Dir),
                size: e.size,
                mtime: e.mtime.and_then(secs_since_epoch),
            })
            .collect())
    }

    async fn stat(&self, path: &str) -> anyhow::Result<FwdStat> {
        if path == "/" {
            return Ok(FwdStat {
                exists: true,
                is_dir: true,
                size: 0,
                mtime: None,
            });
        }
        let Some((res, vp)) = self.route(path) else {
            return Ok(FwdStat::missing());
        };
        Ok(match res.stat(&vp).await {
            Ok(s) => FwdStat {
                exists: true,
                is_dir: matches!(s.kind, FileKind::Dir),
                size: s.size,
                mtime: s.mtime.and_then(secs_since_epoch),
            },
            Err(_) => FwdStat::missing(),
        })
    }

    async fn read(
        &self,
        path: &str,
        offset: Option<u64>,
        size: Option<u64>,
    ) -> anyhow::Result<Vec<u8>> {
        let (res, vp) = self
            .route(path)
            .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
        let range = match (offset, size) {
            (Some(o), Some(s)) => Some(o..o + s),
            _ => None,
        };
        // The vendored core returns a typed `VfsError`; collapse it into `anyhow`
        // for the HTTP layer.
        res.read_bytes(&vp, range)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    async fn write(&self, path: &str, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        let (res, vp) = self
            .route(path)
            .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
        // A `.cmd/<op>` path invokes a provider domain command and returns its
        // JSON result; a normal path writes bytes.
        if let Some(op) = vp.as_str().strip_prefix("/.cmd/") {
            res.command(op, &data)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
        } else {
            res.write_bytes(&vp, data)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(b"{\"ok\":true}".to_vec())
        }
    }

    async fn unlink(&self, path: &str) -> anyhow::Result<()> {
        let (res, vp) = self
            .route(path)
            .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
        res.unlink(&vp).await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    async fn mkdir(&self, path: &str) -> anyhow::Result<()> {
        let (res, vp) = self
            .route(path)
            .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
        res.mkdir(&vp).await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    async fn rmdir(&self, path: &str) -> anyhow::Result<()> {
        let (res, vp) = self
            .route(path)
            .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
        res.rmdir(&vp).await.map_err(|e| anyhow::anyhow!("{e}"))
    }

    async fn rename(&self, from: &str, to: &str) -> anyhow::Result<()> {
        let (res, from_vp) = self
            .route(from)
            .ok_or_else(|| anyhow::anyhow!("no mount for {from}"))?;
        let (_, to_vp) = self
            .route(to)
            .ok_or_else(|| anyhow::anyhow!("no mount for {to}"))?;
        res.rename(&from_vp, &to_vp)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}
