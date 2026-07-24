use std::ops::Range;

use async_trait::async_trait;

use crate::vfs::error::{ResourceError, ResourceResult};
use crate::vfs::path::MountPath;

/// Virtual top-level dir exposing a provider's server-side search — see
/// [`Resource::search`]. Exists only on mounts whose `supports_search()` is
/// true; elsewhere the path resolves like any other (usually not found).
pub(crate) const SEARCH_DIR: &str = ".search";

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
    /// Birth/creation time per entry, if the backend reports one (local files);
    /// `None` for providers that don't distinguish a birth time.
    pub created: Option<std::time::SystemTime>,
    /// Strong version tag from the listing (S3 `ETag`), if the backend reports
    /// one. Carried so the stat fast-path can pin a read to it (`If-Match`)
    /// instead of validating by the coarser mtime.
    pub etag: Option<String>,
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
    /// Birth/creation time, if the backend reports one (local files' `created`);
    /// `None` for providers that don't distinguish a birth time.
    pub created: Option<std::time::SystemTime>,
    /// Entity tag / content fingerprint, if available (S3 `ETag`).
    pub etag: Option<String>,
    /// Version id, if the backend is versioned (S3 `VersionId`).
    pub version: Option<String>,
}

/// A mounted provider. One instance owns one set of credentials, so the same
/// provider type mounted with different credentials is distinct instances.
///
/// Frontends translate filesystem callbacks into these operations. Errors are
/// the typed [`ResourceError`]; callers (the WebDAV layer) map them to protocol
/// errors — most importantly [`ResourceError::NotFound`] onto a 404.
#[async_trait]
pub trait Resource: Send + Sync {
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<Range<u64>>,
    ) -> ResourceResult<Vec<u8>>;

    /// Read `range` validated against a caller-pinned snapshot (`stat` captured
    /// once when the read began), so every chunk of one read stays consistent
    /// without a per-chunk stat. Default: ignore the pin and read normally.
    async fn read_bytes_pinned(
        &self,
        path: &MountPath,
        range: Option<Range<u64>>,
        _stat: &FileStat,
    ) -> ResourceResult<Vec<u8>> {
        self.read_bytes(path, range).await
    }

    async fn write_bytes(&self, path: &MountPath, data: Vec<u8>) -> ResourceResult<()>;

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>>;

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat>;

    async fn unlink(&self, path: &MountPath) -> ResourceResult<()> {
        let _ = path;
        Err(ResourceError::Unsupported)
    }

    /// Create a directory at `path`. Default: unsupported.
    async fn mkdir(&self, path: &MountPath) -> ResourceResult<()> {
        let _ = path;
        Err(ResourceError::Unsupported)
    }

    /// Remove the directory (and its contents) at `path`. Default: unsupported.
    async fn rmdir(&self, path: &MountPath) -> ResourceResult<()> {
        let _ = path;
        Err(ResourceError::Unsupported)
    }

    /// Rename/move `from` to `to`. Default: unsupported.
    async fn rename(&self, from: &MountPath, to: &MountPath) -> ResourceResult<()> {
        let _ = (from, to);
        Err(ResourceError::Unsupported)
    }

    /// Domain operation routed from a `/<mount>/.cmd/<name>` write
    /// (e.g. Notion `page-create`, GDocs `docs-append`).
    async fn command(&self, name: &str, body: &[u8]) -> ResourceResult<Vec<u8>> {
        let _ = (name, body);
        Err(ResourceError::Unsupported)
    }

    /// System-prompt section describing this mount's layout and commands.
    fn prompt(&self) -> &str {
        ""
    }

    /// Whether this provider implements [`Resource::search`]. Gates the
    /// virtual `.search/` tree: on mounts without server-side search the path
    /// simply doesn't exist (their prompts don't mention it), so the agent is
    /// never taught a convention that would fail there. Default: `false`.
    fn supports_search(&self) -> bool {
        false
    }

    /// Server-side search — the entries of a virtual `.search/<query>/`
    /// listing. The query is passed verbatim in the provider's native syntax
    /// (each mount's prompt documents its own); one path segment, so it can
    /// contain spaces but never `/`. Returned entries must resolve *below*
    /// `.search/<query>/` through the provider's own readdir/stat/read (i.e.
    /// address results by id, not by tree position). The top-level `.search`
    /// routing lives in the metadata-cache wrapper; providers only implement
    /// this and their sub-paths. Default: unsupported.
    async fn search(&self, query: &str) -> ResourceResult<Vec<DirEntry>> {
        let _ = query;
        Err(ResourceError::Unsupported)
    }

    /// Whether `readdir` returns a *complete* listing of a directory. When true,
    /// a fresh parent listing that lacks a name proves that name doesn't exist,
    /// so the cache can answer `stat` of a missing child with `NotFound` without
    /// a network probe (negative caching). Gmail listings come from a TTL-cached
    /// index (optionally capped via `index_cap`), so a date/message absent from
    /// a listing may still exist and must be probed — it returns `false`.
    /// Default: `true`.
    fn listings_complete(&self) -> bool {
        true
    }
}
