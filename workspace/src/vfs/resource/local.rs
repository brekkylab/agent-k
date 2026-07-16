//! The workspace's local file tree as a [`Resource`], so it routes through the
//! same mount machinery as the external-provider mounts (S3, Notion) instead of
//! a separate `tokio::fs` branch. Mounted at the reserved `/files` prefix.
//!
//! Stateless per call (open→op→close), matching the `Resource` contract: reads
//! pull a byte range, writes replace the whole object. Reports the local
//! timestamps the WebDAV layer wants — including birth time via
//! [`FileStat::created`], which is why that field exists on the provider stat.

use std::io::SeekFrom;
use std::ops::Range;
use std::path::PathBuf;
use std::time::SystemTime;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::vfs::{
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

/// The local file tree rooted at `root` (a workspace's on-disk `…/files` dir).
pub struct LocalResource {
    root: PathBuf,
}

impl LocalResource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolve a mount-relative [`MountPath`] to an on-disk path under `root`,
    /// rejecting `.`/`..` segments so a mount can't address outside its root
    /// (mirrors `S3Resource::os_path`). An escaping path can't name a node →
    /// `NotFound`.
    fn resolve(&self, path: &MountPath) -> ResourceResult<PathBuf> {
        let mut out = self.root.clone();
        for seg in path.as_str().split('/').filter(|s| !s.is_empty()) {
            if seg == "." || seg == ".." {
                return Err(ResourceError::NotFound);
            }
            out.push(seg);
        }
        Ok(out)
    }
}

/// Map an I/O error to the typed [`ResourceError`], preserving `NotFound` so the
/// WebDAV layer answers 404 (the blanket `From<io::Error>` collapses everything
/// to `Backend`, which would surface as a 500).
fn io_err(e: std::io::Error) -> ResourceError {
    if e.kind() == std::io::ErrorKind::NotFound {
        ResourceError::NotFound
    } else {
        ResourceError::Backend(e.into())
    }
}

/// POSIX change time (`ctime`) of a local node, where the platform exposes it.
#[cfg(unix)]
fn ctime_of(m: &std::fs::Metadata) -> Option<SystemTime> {
    use std::os::unix::fs::MetadataExt;
    use std::time::{Duration, UNIX_EPOCH};
    Some(UNIX_EPOCH + Duration::new(m.ctime() as u64, 0))
}

#[cfg(not(unix))]
fn ctime_of(_m: &std::fs::Metadata) -> Option<SystemTime> {
    None
}

fn stat_of(m: &std::fs::Metadata) -> FileStat {
    FileStat {
        kind: if m.is_dir() {
            FileKind::Dir
        } else {
            FileKind::File
        },
        size: m.len(),
        mtime: m.modified().ok(),
        atime: m.accessed().ok(),
        ctime: ctime_of(m),
        created: m.created().ok(),
        etag: None,
        version: None,
    }
}

#[async_trait]
impl Resource for LocalResource {
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        let p = self.resolve(path)?;
        let mut f = tokio::fs::File::open(&p).await.map_err(io_err)?;
        let mut buf = Vec::new();
        match range {
            // Bounded read: seek to the start, then read at most `len` bytes. A
            // start at/after EOF reads nothing (empty) rather than erroring,
            // matching the S3 provider's range-past-EOF behaviour.
            Some(r) => {
                f.seek(SeekFrom::Start(r.start)).await.map_err(io_err)?;
                let len = r.end.saturating_sub(r.start);
                (&mut f)
                    .take(len)
                    .read_to_end(&mut buf)
                    .await
                    .map_err(io_err)?;
            }
            None => {
                f.read_to_end(&mut buf).await.map_err(io_err)?;
            }
        }
        Ok(buf)
    }

    async fn write_bytes(&self, path: &MountPath, data: Vec<u8>) -> ResourceResult<()> {
        let p = self.resolve(path)?;
        let mut oo = tokio::fs::OpenOptions::new();
        oo.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            oo.mode(0o600);
        }
        let mut f = oo.open(&p).await.map_err(io_err)?;
        f.write_all(&data).await.map_err(io_err)?;
        f.flush().await.map_err(io_err)?;
        Ok(())
    }

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        let p = self.resolve(path)?;
        let mut rd = tokio::fs::read_dir(&p).await.map_err(io_err)?;
        let mut out = Vec::new();
        while let Some(entry) = rd.next_entry().await.map_err(io_err)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Per-entry metadata (no symlink follow — a symlink lists as a file).
            let entry = match entry.metadata().await {
                Ok(m) => {
                    let s = stat_of(&m);
                    DirEntry {
                        name,
                        kind: s.kind,
                        size: s.size,
                        mtime: s.mtime,
                        atime: s.atime,
                        ctime: s.ctime,
                        created: s.created,
                    }
                }
                Err(_) => DirEntry {
                    name,
                    kind: FileKind::File,
                    size: 0,
                    mtime: None,
                    atime: None,
                    ctime: None,
                    created: None,
                },
            };
            out.push(entry);
        }
        Ok(out)
    }

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
        // Follows symlinks (symlink kind is not modelled — it resolves to its
        // target), matching the previous local `metadata`.
        let m = tokio::fs::metadata(self.resolve(path)?)
            .await
            .map_err(io_err)?;
        Ok(stat_of(&m))
    }

    async fn unlink(&self, path: &MountPath) -> ResourceResult<()> {
        tokio::fs::remove_file(self.resolve(path)?)
            .await
            .map_err(io_err)
    }

    async fn mkdir(&self, path: &MountPath) -> ResourceResult<()> {
        let p = self.resolve(path)?;
        let mut b = tokio::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(&p).await.map_err(io_err)
    }

    async fn rmdir(&self, path: &MountPath) -> ResourceResult<()> {
        tokio::fs::remove_dir(self.resolve(path)?)
            .await
            .map_err(io_err)
    }

    async fn rename(&self, from: &MountPath, to: &MountPath) -> ResourceResult<()> {
        let (from, to) = (self.resolve(from)?, self.resolve(to)?);
        rename_compat(&from, &to).await.map_err(io_err)
    }
}

/// Rename `from` to `to`. WebDAV permits renaming a directory over an existing
/// file, which `rename` rejects (`ENOTDIR`); detect that and retry after
/// removing the destination file (mirrors `dav_server`'s `LocalFs`).
async fn rename_compat(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    match tokio::fs::rename(from, to).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let from_is_dir = tokio::fs::metadata(from)
                .await
                .map(|m| m.is_dir())
                .unwrap_or(false);
            let to_is_file = tokio::fs::metadata(to)
                .await
                .map(|m| m.is_file())
                .unwrap_or(false);
            if from_is_dir && to_is_file {
                let _ = tokio::fs::remove_file(to).await;
                tokio::fs::rename(from, to).await
            } else {
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res() -> (tempfile::TempDir, LocalResource) {
        let tmp = tempfile::tempdir().unwrap();
        let r = LocalResource::new(tmp.path().to_path_buf());
        (tmp, r)
    }

    #[tokio::test]
    async fn round_trip_write_stat_read() {
        let (_tmp, r) = res();
        let p = MountPath::new("/note.txt");
        r.write_bytes(&p, b"hello local".to_vec()).await.unwrap();

        let st = r.stat(&p).await.unwrap();
        assert!(matches!(st.kind, FileKind::File));
        assert_eq!(st.size, 11);
        // Local files carry birth time (the reason FileStat has `created`).
        assert!(st.created.is_some());
        assert!(st.mtime.is_some());

        assert_eq!(r.read_bytes(&p, None).await.unwrap(), b"hello local");
        assert_eq!(r.read_bytes(&p, Some(6..11)).await.unwrap(), b"local");
        // A range starting at/after EOF reads empty, not an error.
        assert!(r.read_bytes(&p, Some(11..20)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn readdir_and_dir_ops() {
        let (_tmp, r) = res();
        r.mkdir(&MountPath::new("/sub")).await.unwrap();
        r.write_bytes(&MountPath::new("/sub/a.txt"), b"x".to_vec())
            .await
            .unwrap();

        let names: Vec<_> = r
            .readdir(&MountPath::new("/sub"))
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["a.txt".to_string()]);

        r.rename(&MountPath::new("/sub/a.txt"), &MountPath::new("/sub/b.txt"))
            .await
            .unwrap();
        assert!(matches!(
            r.stat(&MountPath::new("/sub/a.txt")).await,
            Err(ResourceError::NotFound)
        ));
        assert_eq!(r.stat(&MountPath::new("/sub/b.txt")).await.unwrap().size, 1);

        r.unlink(&MountPath::new("/sub/b.txt")).await.unwrap();
        r.rmdir(&MountPath::new("/sub")).await.unwrap();
        assert!(matches!(
            r.stat(&MountPath::new("/sub")).await,
            Err(ResourceError::NotFound)
        ));
    }

    #[tokio::test]
    async fn missing_path_is_not_found() {
        let (_tmp, r) = res();
        assert!(matches!(
            r.stat(&MountPath::new("/nope.txt")).await,
            Err(ResourceError::NotFound)
        ));
        assert!(matches!(
            r.read_bytes(&MountPath::new("/nope.txt"), None).await,
            Err(ResourceError::NotFound)
        ));
    }

    #[tokio::test]
    async fn dot_segments_are_rejected() {
        let (_tmp, r) = res();
        for bad in ["/../up", "/a/../b", "/."] {
            assert!(
                matches!(
                    r.stat(&MountPath::new(bad)).await,
                    Err(ResourceError::NotFound)
                ),
                "should reject {bad:?}"
            );
        }
    }
}
