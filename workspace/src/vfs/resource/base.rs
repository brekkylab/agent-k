use std::ops::Range;

use async_trait::async_trait;

use crate::vfs::error::{ResourceError, ResourceResult};
use crate::vfs::path::MountPath;

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
    /// What the entry's *subject* is, when the backend knows better than the
    /// filename does. A provider that serves a rendering rather than the
    /// original bytes has to say so somewhere: a Google Doc is served as
    /// `report.gdoc.json`, so a client guessing from the extension only ever
    /// learns `application/json`, never which of the three document types it is —
    /// and the Drive names those come from carry no extension at all. `None` =
    /// nothing better than the name to go on.
    pub content_type: Option<String>,
    /// See [`FileStat::size_is_estimate`]. Carried from the listing so the value
    /// survives `readdir` → cache → `stat`.
    pub size_is_estimate: bool,
    /// See [`FileStat::serves_whole`]. Carried for the same reason: the metadata cache
    /// answers a `stat` after a `readdir` from this row, and the read strategy inverts
    /// on this flag.
    pub serves_whole: bool,
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
    /// See [`DirEntry::content_type`].
    pub content_type: Option<String>,
    /// `size` is an upper bound, not the real length: the provider cannot know it
    /// without producing the content (a Drive document conversion). Listings and
    /// `stat` keep the estimate — sizing every row would fetch every row — but a
    /// consumer that must be exact, like a WebDAV `GET` filling in
    /// `Content-Length`, resolves it when the file is opened.
    pub size_is_estimate: bool,
    /// Whether asking for *part* of this node is the same work as asking for all of
    /// it: `true` for one the backend builds on request (a Google Doc, a Notion
    /// page), where `head -c 100` builds the whole document and returns a hundred
    /// bytes of it.
    ///
    /// The read strategy inverts on this. A ranged node that misses the cache
    /// re-reads one window, so only small ones are worth keeping; a whole-only node
    /// re-builds everything, so it is fetched whole and kept up to the entire budget
    /// — without that, a 10 MB document read in 256 KB chunks builds itself forty
    /// times.
    ///
    /// Independent of [`Self::size_is_estimate`]: a file whose length Drive never
    /// reported is unsized but still ranged, and reading its placeholder as grounds
    /// to fetch everything turned a 256 KB read of a 2 GB object into a 2 GB
    /// download.
    pub serves_whole: bool,
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

    /// Whether a `stat` may *produce* a file to learn a length the listing could only
    /// guess at (a 0 or a placeholder — see [`FileStat::size`] and
    /// [`FileStat::size_is_estimate`]).
    ///
    /// `true` (the default) keeps `stat`/`HEAD`/`PROPFIND` exact for providers that
    /// render on read: Notion's `page.json` is one file per page directory, so one
    /// render per `ls -l` is a fair trade for a correct length.
    ///
    /// `false` is for providers where that trade inverts. Drive serves a document per
    /// file and each takes seconds to build, so `ls -l` in a folder of them would build
    /// every one to print a number. Such a provider's listing stands as it is, and the
    /// length becomes exact when something reads the file. Which placeholder it uses is
    /// its own choice: Notion reports 0, Drive reports an upper bound marked
    /// `size_is_estimate`, because a 0 reads as "empty" and search tools skip those.
    fn resolve_size_on_stat(&self) -> bool {
        true
    }

    /// Whether this provider ever fills [`DirEntry::content_type`].
    ///
    /// A capability answer, not a per-path one, so a caller can decide *without
    /// a stat* whether asking is worth it. The WebDAV layer needs exactly that:
    /// it has to serve the type as a dead property, and dav-server asks for dead
    /// properties one node at a time — so a provider that never has an answer
    /// must be able to say so up front rather than pay a `metadata()` per entry
    /// to return nothing.
    fn reports_content_type(&self) -> bool {
        false
    }
}
