use std::ops::Range;

use async_trait::async_trait;

use crate::vfs::error::{VfsError, VfsResult};
use crate::vfs::path::VPath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FileKind {
    #[default]
    File,
    Dir,
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub kind: FileKind,
    pub size: u64,
    /// Last-modified time, if the backend reports one per entry (S3
    /// `LastModified`, Notion `last_edited_time`). Carried from `readdir` so the
    /// cache can serve it on the stat fast-path (`ls -l`) instead of falling
    /// back to the UNIX epoch (R2).
    pub mtime: Option<std::time::SystemTime>,
    /// Last-access time per entry, if the backend reports one (usually `None`).
    pub atime: Option<std::time::SystemTime>,
    /// Change/creation time per entry (Notion `created_time`), if reported.
    pub ctime: Option<std::time::SystemTime>,
}

#[derive(Clone, Debug, Default)]
pub struct FileStat {
    pub kind: FileKind,
    pub size: u64,
    /// Last-modified time, if the backend reports one (S3 `LastModified`,
    /// Notion `last_edited_time`).
    pub mtime: Option<std::time::SystemTime>,
    /// Last-access time, if the backend reports one. Most object/document
    /// backends don't track access time, so this is usually `None`.
    pub atime: Option<std::time::SystemTime>,
    /// Change/creation time, if the backend reports one (Notion `created_time`).
    /// Not POSIX `ctime` exactly — the nearest timestamp the backend exposes.
    pub ctime: Option<std::time::SystemTime>,
    /// Entity tag / content fingerprint, if available (S3 `ETag`).
    pub etag: Option<String>,
    /// Version id, if the backend is versioned (S3 `VersionId`).
    pub version: Option<String>,
}

/// A mounted provider. One instance owns one set of credentials, so the same
/// provider type mounted with different credentials is distinct instances.
///
/// Frontends translate filesystem callbacks into these operations. Errors are
/// the typed [`VfsError`]; callers (the WebDAV layer) map them to protocol
/// errors — most importantly [`VfsError::NotFound`] onto a 404.
#[async_trait]
pub trait Resource: Send + Sync {
    async fn read_bytes(&self, path: &VPath, range: Option<Range<u64>>) -> VfsResult<Vec<u8>>;

    async fn write_bytes(&self, path: &VPath, data: Vec<u8>) -> VfsResult<()>;

    async fn readdir(&self, path: &VPath) -> VfsResult<Vec<DirEntry>>;

    async fn stat(&self, path: &VPath) -> VfsResult<FileStat>;

    async fn unlink(&self, path: &VPath) -> VfsResult<()> {
        let _ = path;
        Err(VfsError::Unsupported)
    }

    /// Create a directory at `path`. Default: unsupported.
    async fn mkdir(&self, path: &VPath) -> VfsResult<()> {
        let _ = path;
        Err(VfsError::Unsupported)
    }

    /// Remove the directory (and its contents) at `path`. Default: unsupported.
    async fn rmdir(&self, path: &VPath) -> VfsResult<()> {
        let _ = path;
        Err(VfsError::Unsupported)
    }

    /// Rename/move `from` to `to`. Default: unsupported.
    async fn rename(&self, from: &VPath, to: &VPath) -> VfsResult<()> {
        let _ = (from, to);
        Err(VfsError::Unsupported)
    }

    /// Domain operation routed from a `/<mount>/.cmd/<name>` write
    /// (e.g. Notion `page-create`, GDocs `docs-append`).
    async fn command(&self, name: &str, body: &[u8]) -> VfsResult<Vec<u8>> {
        let _ = (name, body);
        Err(VfsError::Unsupported)
    }

    /// System-prompt section describing this mount's layout and commands.
    fn prompt(&self) -> &str {
        ""
    }
}
