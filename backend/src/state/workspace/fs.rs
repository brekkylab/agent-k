//! Protocol-agnostic filesystem primitives for a workspace.
//!
//! These types ([`WorkspaceFs`], [`File`], [`DirEntry`], …)
//! mirror the operations a WebDAV backend needs but carry **no** dependency on
//! `dav_server`: they speak in workspace-relative path strings, owned byte
//! buffers, and `std`/`tokio` types. The WebDAV protocol layer
//! (see [`crate::router`]) adapts these onto `dav_server`'s traits.
//!
//! Every mutating operation also performs the workspace's own side-processing
//! (today: `knowledge/` ingestion, currently a logging stub); that
//! classification used to live in the router and now lives here, so that
//! *every* caller of the filesystem — not just the WebDAV one — observes the
//! same effects.

use std::io::{self, SeekFrom};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use futures_util::{Stream, StreamExt as _, stream};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;

use crate::vfs::sandbox::{ForwardFs, FwdEntry, FwdStat, secs_since_epoch};
use crate::vfs::{FileKind, FileStat, Resource, VPath, Vfs, VfsError};

/// Errors a workspace filesystem operation can produce. A protocol-agnostic
/// subset that the WebDAV layer maps onto `dav_server::fs::FsError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    /// Operation not supported on this platform / for this node.
    NotImplemented,
    /// Catch-all failure.
    GeneralFailure,
    /// Tried to create something that already exists.
    Exists,
    /// Path not found.
    NotFound,
    /// Operation not permitted.
    Forbidden,
}

/// Result alias for filesystem operations.
pub type FsResult<T> = Result<T, FsError>;

impl From<io::Error> for FsError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => FsError::NotFound,
            io::ErrorKind::AlreadyExists => FsError::Exists,
            io::ErrorKind::PermissionDenied => FsError::Forbidden,
            _ => FsError::GeneralFailure,
        }
    }
}

/// How a file should be opened. Mirrors the subset of WebDAV `OpenOptions`
/// the workspace honours.
#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub truncate: bool,
    pub create: bool,
    pub create_new: bool,
}

/// Optimisation hint for [`WorkspaceFs::read_dir`]: whether per-entry metadata
/// should follow symlinks ([`Self::Data`]) or describe the link itself.
#[derive(Debug, Clone, Copy)]
pub enum ReadDirMeta {
    /// Entry metadata follows symlinks.
    Data,
    /// Entry metadata describes the symlink itself.
    DataSymlink,
    /// No optimisation; behaves like [`Self::DataSymlink`].
    None,
}

/// The kind of a filesystem node. External providers only ever report
/// [`File`](NodeKind::File) / [`Dir`](NodeKind::Dir); [`Symlink`](NodeKind::Symlink)
/// arises only from a local `symlink_metadata`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
}

/// Protocol-agnostic file metadata: the fields the WebDAV layer reads, decoupled
/// from [`std::fs::Metadata`] so a node can equally be a local file or an
/// external provider object. Timestamps and `executable` are `Option` because
/// non-local sources (S3, Notion, …) don't report them; the WebDAV adapter maps
/// a `None` onto the corresponding `NotImplemented`.
#[derive(Debug, Clone)]
pub struct Stat {
    pub kind: NodeKind,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub status_changed: Option<SystemTime>,
    pub executable: Option<bool>,
}

impl Stat {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, NodeKind::Dir)
    }

    pub fn is_file(&self) -> bool {
        matches!(self.kind, NodeKind::File)
    }

    pub fn is_symlink(&self) -> bool {
        matches!(self.kind, NodeKind::Symlink)
    }
}

impl From<std::fs::Metadata> for Stat {
    fn from(m: std::fs::Metadata) -> Self {
        let kind = if m.is_dir() {
            NodeKind::Dir
        } else if m.is_symlink() {
            NodeKind::Symlink
        } else {
            NodeKind::File
        };
        Stat {
            kind,
            len: m.len(),
            modified: m.modified().ok(),
            accessed: m.accessed().ok(),
            created: m.created().ok(),
            status_changed: status_changed_of(&m),
            executable: executable_of(&m),
        }
    }
}

/// Change time (`ctime`) of a local file, on platforms that expose it.
#[cfg(unix)]
fn status_changed_of(m: &std::fs::Metadata) -> Option<SystemTime> {
    use std::os::unix::fs::MetadataExt;
    use std::time::{Duration, UNIX_EPOCH};
    Some(UNIX_EPOCH + Duration::new(m.ctime() as u64, 0))
}

#[cfg(not(unix))]
fn status_changed_of(_m: &std::fs::Metadata) -> Option<SystemTime> {
    None
}

/// Whether a local *file* has the owner-execute bit set. `None` for non-files
/// and on platforms without a unix mode.
#[cfg(unix)]
fn executable_of(m: &std::fs::Metadata) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt;
    if m.is_file() {
        Some((m.permissions().mode() & 0o100) > 0)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn executable_of(_m: &std::fs::Metadata) -> Option<bool> {
    None
}

/// Map a VFS [`FileKind`] onto the local [`NodeKind`].
fn node_kind_of(kind: FileKind) -> NodeKind {
    match kind {
        FileKind::Dir => NodeKind::Dir,
        FileKind::File => NodeKind::File,
    }
}

/// Build a [`Stat`] from an external provider's [`FileStat`]. External sources
/// report only kind / size / mtime; the local-only projections stay `None`.
fn stat_from_vfs(fs: FileStat) -> Stat {
    Stat {
        kind: node_kind_of(fs.kind),
        len: fs.size,
        modified: fs.mtime,
        accessed: fs.atime,
        created: None,
        status_changed: fs.ctime,
        executable: None,
    }
}

/// Map a provider [`VfsError`] onto the workspace [`FsError`]. The provider
/// classifies the failure itself (typed, not by message), so this is a direct
/// translation — `NotFound` → 404, an unsupported op → `NotImplemented`, and
/// any backend failure → a generic error.
impl From<VfsError> for FsError {
    fn from(e: VfsError) -> Self {
        match e {
            VfsError::NotFound => FsError::NotFound,
            VfsError::Unsupported => FsError::NotImplemented,
            VfsError::Backend(_) => FsError::GeneralFailure,
        }
    }
}

/// True when `rel_path` lives under the `knowledge/` directory. Component-wise
/// match on the leading dir: `knowledge/x` hits, `knowledgebase/x` does not.
fn is_knowledge(rel_path: &str) -> bool {
    rel_path.trim_start_matches('/').starts_with("knowledge/")
}

// Side-processing for `knowledge/` mutations. Stubs for now: the real ingestion
// (parsing, indexing into / dropping from the knowledge store) lands later;
// today they only log. `path` is the absolute on-disk path of the affected file.

/// A *new* file appeared (a write/copy/move landing at a previously-absent path).
fn knowledge_inserted(wid: Uuid, path: &Path) {
    tracing::info!("insert_knowledge (workspace={wid}, path={})", path.display());
}

/// An existing file was overwritten in place.
fn knowledge_updated(wid: Uuid, path: &Path) {
    tracing::info!("update_knowledge (workspace={wid}, path={})", path.display());
}

/// A file left `knowledge/` (a delete, or a move whose source was under it).
/// The file is already gone from disk; `path` is where it lived.
fn knowledge_removed(wid: Uuid, path: &Path) {
    tracing::info!("remove_knowledge (workspace={wid}, path={})", path.display());
}

/// One entry yielded by [`WorkspaceFs::read_dir`]. Metadata is captured eagerly
/// at listing time.
pub struct DirEntry {
    name: Vec<u8>,
    metadata: FsResult<Stat>,
}

impl DirEntry {
    /// Raw filename bytes (no path).
    pub fn name(&self) -> Vec<u8> {
        self.name.clone()
    }

    /// Metadata captured when the directory was listed.
    pub fn metadata(&self) -> FsResult<Stat> {
        self.metadata.clone()
    }
}

/// A stream of directory entries.
pub type DirStream = Pin<Box<dyn Stream<Item = FsResult<DirEntry>> + Send>>;

/// Tracks an in-flight write so its completion (`flush` after a `write_*`) can
/// be reported to the workspace's side-processing.
struct Observer {
    wid: Uuid,
    /// Absolute on-disk path of the file, for the knowledge handlers.
    path: PathBuf,
    /// Whether the file already existed when opened — distinguishes an insert
    /// from an overwrite at flush time.
    existed: bool,
    wrote: bool,
}

/// An open workspace file: either a local on-disk file or a read-only view onto
/// a mounted external-provider object. The WebDAV layer holds one behind the
/// same interface regardless of which it is.
pub enum File {
    /// A local on-disk file. Readable / writable / seekable, like
    /// [`std::fs::File`], and — for write opens — reports a completed write to
    /// the workspace once `flush` follows at least one `write_*`.
    Local(LocalFile),
    /// A read-only cursor over a mounted provider object.
    Mount(MountFile),
}

/// A local on-disk file (the original workspace-file behaviour).
pub struct LocalFile {
    file: tokio::fs::File,
    buf: BytesMut,
    observer: Option<Observer>,
}

/// A read-only cursor over a mounted external-provider object. Reads pull byte
/// ranges from the [`Resource`] on demand and advance the cursor; writes are
/// rejected because mounts are read-only.
pub struct MountFile {
    resource: Arc<dyn Resource>,
    path: VPath,
    offset: u64,
}

impl File {
    pub async fn metadata(&mut self) -> FsResult<Stat> {
        match self {
            File::Local(f) => f
                .file
                .metadata()
                .await
                .map(Stat::from)
                .map_err(FsError::from),
            File::Mount(m) => m
                .resource
                .stat(&m.path)
                .await
                .map(stat_from_vfs)
                .map_err(FsError::from),
        }
    }

    pub async fn write_bytes(&mut self, buf: Bytes) -> FsResult<()> {
        match self {
            File::Local(f) => {
                if let Some(o) = f.observer.as_mut() {
                    o.wrote = true;
                }
                f.file.write_all(&buf).await.map_err(FsError::from)
            }
            File::Mount(_) => Err(FsError::Forbidden),
        }
    }

    pub async fn write_buf(&mut self, mut buf: Box<dyn Buf + Send>) -> FsResult<()> {
        match self {
            File::Local(f) => {
                if let Some(o) = f.observer.as_mut() {
                    o.wrote = true;
                }
                while buf.has_remaining() {
                    let n = f.file.write(buf.chunk()).await.map_err(FsError::from)?;
                    buf.advance(n);
                }
                Ok(())
            }
            File::Mount(_) => Err(FsError::Forbidden),
        }
    }

    pub async fn read_bytes(&mut self, count: usize) -> FsResult<Bytes> {
        match self {
            File::Local(f) => {
                // Reuse `f.buf`'s allocation across reads; cap the read at
                // `count` and hand back exactly the bytes filled (empty at EOF).
                let mut buf = std::mem::take(&mut f.buf);
                buf.reserve(count);
                let res = (&mut f.file).take(count as u64).read_buf(&mut buf).await;
                f.buf = buf;
                res.map_err(FsError::from)?;
                Ok(f.buf.split().freeze())
            }
            File::Mount(m) => {
                // Pull the next `count` bytes from the provider and advance the
                // cursor. A short read (or empty at EOF) is honoured verbatim.
                let end = m.offset.saturating_add(count as u64);
                let data = m
                    .resource
                    .read_bytes(&m.path, Some(m.offset..end))
                    .await
                    .map_err(FsError::from)?;
                m.offset = m.offset.saturating_add(data.len() as u64);
                Ok(Bytes::from(data))
            }
        }
    }

    pub async fn seek(&mut self, pos: SeekFrom) -> FsResult<u64> {
        match self {
            File::Local(f) => f.file.seek(pos).await.map_err(FsError::from),
            File::Mount(m) => {
                let new = match pos {
                    SeekFrom::Start(n) => n,
                    SeekFrom::Current(d) => (m.offset as i64 + d).max(0) as u64,
                    // Seeking from the end needs the object size; one stat serves it.
                    SeekFrom::End(d) => {
                        let size = m.resource.stat(&m.path).await.map_err(FsError::from)?.size;
                        (size as i64 + d).max(0) as u64
                    }
                };
                m.offset = new;
                Ok(new)
            }
        }
    }

    pub async fn flush(&mut self) -> FsResult<()> {
        match self {
            File::Local(f) => {
                f.file.flush().await?;
                if let Some(o) = f.observer.as_mut()
                    && o.wrote
                {
                    // Clear first so a second flush on the same handle won't re-report.
                    o.wrote = false;
                    if o.existed {
                        knowledge_updated(o.wid, &o.path);
                    } else {
                        knowledge_inserted(o.wid, &o.path);
                    }
                }
                Ok(())
            }
            // Nothing buffered on a read-only mount cursor.
            File::Mount(_) => Ok(()),
        }
    }
}

/// A filesystem handle scoped to a single workspace. Cheap to clone
/// (just an owned root path and workspace id); `Send + Sync + 'static`, so the
/// WebDAV layer can hold one for the lifetime of a request.
#[derive(Clone)]
pub struct WorkspaceFs {
    root: PathBuf,
    wid: Uuid,
    /// External-provider mounts, if any. Paths under a mount prefix are served
    /// by the provider (read-only); every other path stays on local disk.
    vfs: Option<Arc<Vfs>>,
}

impl WorkspaceFs {
    pub(super) fn new(root: PathBuf, wid: Uuid) -> Self {
        Self {
            root,
            wid,
            vfs: None,
        }
    }

    /// Attach a set of external-provider mounts. Paths under a mount prefix are
    /// routed to the provider; all others stay local. `None` clears mounts.
    pub(super) fn with_vfs(mut self, vfs: Option<Arc<Vfs>>) -> Self {
        self.vfs = vfs;
        self
    }

    /// Route a workspace-relative path to the mount that owns it, if any.
    /// `Some((resource, vpath))` means the path lives under an external
    /// provider; `None` means it stays on local disk.
    fn route(&self, rel_path: &str) -> Option<(Arc<dyn Resource>, VPath)> {
        let vfs = self.vfs.as_ref()?;
        let abs = if rel_path.starts_with('/') {
            rel_path.to_string()
        } else {
            format!("/{rel_path}")
        };
        vfs.route(&abs).map(|(r, p)| (Arc::clone(r), p))
    }

    /// Top-level mount names (no leading `/`), surfaced as virtual subdirectories
    /// at the workspace root.
    fn mount_names(&self) -> Vec<String> {
        self.vfs.as_ref().map(|v| v.mount_names()).unwrap_or_default()
    }

    /// Absolute on-disk path of `rel_path` (a workspace-relative path such as
    /// `/knowledge/foo.txt`) inside this workspace.
    fn resolve(&self, rel_path: &str) -> PathBuf {
        self.root.join(rel_path.trim_start_matches('/'))
    }

    pub async fn metadata(&self, rel_path: &str) -> FsResult<Stat> {
        if let Some((resource, vpath)) = self.route(rel_path) {
            return resource
                .stat(&vpath)
                .await
                .map(stat_from_vfs)
                .map_err(FsError::from);
        }
        tokio::fs::metadata(self.resolve(rel_path))
            .await
            .map(Stat::from)
            .map_err(FsError::from)
    }

    pub async fn symlink_metadata(&self, rel_path: &str) -> FsResult<Stat> {
        // External providers have no symlinks, so a mount path stats the same
        // as `metadata`.
        if let Some((resource, vpath)) = self.route(rel_path) {
            return resource
                .stat(&vpath)
                .await
                .map(stat_from_vfs)
                .map_err(FsError::from);
        }
        tokio::fs::symlink_metadata(self.resolve(rel_path))
            .await
            .map(Stat::from)
            .map_err(FsError::from)
    }

    pub async fn read_dir(&self, rel_path: &str, meta: ReadDirMeta) -> FsResult<DirStream> {
        // A path under a mount lists through the provider.
        if let Some((resource, vpath)) = self.route(rel_path) {
            let entries = resource.readdir(&vpath).await.map_err(FsError::from)?;
            let out: Vec<FsResult<DirEntry>> =
                entries.into_iter().map(|e| Ok(dir_entry_from_vfs(e))).collect();
            return Ok(Box::pin(stream::iter(out)));
        }

        let path = self.resolve(rel_path);
        let mut rd = tokio::fs::read_dir(&path).await?;
        // Collect eagerly (metadata captured at listing time) and replay as a
        // stream, matching the original contract.
        let mut out: Vec<FsResult<DirEntry>> = Vec::new();
        loop {
            match rd.next_entry().await {
                Ok(Some(entry)) => {
                    let md = match meta {
                        ReadDirMeta::Data => tokio::fs::metadata(entry.path()).await,
                        ReadDirMeta::DataSymlink | ReadDirMeta::None => entry.metadata().await,
                    };
                    out.push(Ok(DirEntry {
                        name: dir_entry_name(&entry),
                        metadata: md.map(Stat::from).map_err(FsError::from),
                    }));
                }
                Ok(None) => break,
                Err(e) => {
                    out.push(Err(FsError::from(e)));
                    break;
                }
            }
        }
        // At the workspace root, surface each mount as a virtual subdirectory
        // alongside the local entries.
        if is_root_path(rel_path) {
            for name in self.mount_names() {
                out.push(Ok(mount_dir_entry(&name)));
            }
        }
        Ok(Box::pin(stream::iter(out)))
    }

    pub async fn open(&self, rel_path: &str, options: OpenOptions) -> FsResult<File> {
        // A mount path opens read-only; any write intent is rejected up front.
        if let Some((resource, vpath)) = self.route(rel_path) {
            let wants_write = options.write
                || options.append
                || options.create
                || options.create_new
                || options.truncate;
            if wants_write {
                return Err(FsError::Forbidden);
            }
            // Confirm the target is a readable object before handing back a
            // cursor, so a GET on a missing path / directory fails cleanly.
            let stat = resource.stat(&vpath).await.map_err(FsError::from)?;
            if matches!(stat.kind, FileKind::Dir) {
                return Err(FsError::Forbidden);
            }
            return Ok(File::Mount(MountFile {
                resource,
                path: vpath,
                offset: 0,
            }));
        }

        let is_write = options.write || options.append || options.create || options.create_new;
        let path = self.resolve(rel_path);
        // Probe before opening: a create/truncating open would make the file
        // exist (or empty) regardless, so capture prior existence here to tell
        // an insert from an overwrite at flush time.
        let existed = if is_write {
            tokio::fs::metadata(&path).await.is_ok()
        } else {
            false
        };
        let file = tokio::fs::OpenOptions::from(open_options_std(&options))
            .open(&path)
            .await
            .map_err(FsError::from)?;
        // Only knowledge writes need observing; the flush hook then reports an
        // insert or update without re-classifying.
        let observer = (is_write && is_knowledge(rel_path)).then_some(Observer {
            wid: self.wid,
            path,
            existed,
            wrote: false,
        });
        Ok(File::Local(LocalFile {
            file,
            buf: BytesMut::new(),
            observer,
        }))
    }

    pub async fn create_dir(&self, rel_path: &str) -> FsResult<()> {
        if self.route(rel_path).is_some() {
            return Err(FsError::Forbidden);
        }
        let path = self.resolve(rel_path);
        let mut builder = tokio::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            builder.mode(0o700);
        }
        builder.create(&path).await.map_err(FsError::from)
    }

    pub async fn remove_dir(&self, rel_path: &str) -> FsResult<()> {
        if self.route(rel_path).is_some() {
            return Err(FsError::Forbidden);
        }
        tokio::fs::remove_dir(self.resolve(rel_path))
            .await
            .map_err(FsError::from)
    }

    pub async fn remove_file(&self, rel_path: &str) -> FsResult<()> {
        if self.route(rel_path).is_some() {
            return Err(FsError::Forbidden);
        }
        let path = self.resolve(rel_path);
        tokio::fs::remove_file(&path).await.map_err(FsError::from)?;
        if is_knowledge(rel_path) {
            knowledge_removed(self.wid, &path);
        }
        Ok(())
    }

    pub async fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        // Mounts are read-only, and cross-boundary moves aren't supported yet;
        // reject a rename touching either side of a mount.
        if self.route(from).is_some() || self.route(to).is_some() {
            return Err(FsError::Forbidden);
        }
        // Probe the destination before the move so we can tell whether it
        // landed on a fresh path (insert) or replaced one (update).
        let to_existed = self.metadata(to).await.is_ok();
        let from_path = self.resolve(from);
        let to_path = self.resolve(to);
        rename_compat(&from_path, &to_path)
            .await
            .map_err(FsError::from)?;
        // The source path left `knowledge/`; the destination arrived in it.
        if is_knowledge(from) {
            knowledge_removed(self.wid, &from_path);
        }
        if is_knowledge(to) {
            if to_existed {
                knowledge_updated(self.wid, &to_path);
            } else {
                knowledge_inserted(self.wid, &to_path);
            }
        }
        Ok(())
    }

    pub async fn copy(&self, from: &str, to: &str) -> FsResult<()> {
        // Copying to/from a mount isn't supported in the read-only phase.
        if self.route(from).is_some() || self.route(to).is_some() {
            return Err(FsError::Forbidden);
        }
        let to_existed = self.metadata(to).await.is_ok();
        let from_path = self.resolve(from);
        let to_path = self.resolve(to);
        tokio::fs::copy(&from_path, &to_path)
            .await
            .map_err(FsError::from)?;
        if is_knowledge(to) {
            if to_existed {
                knowledge_updated(self.wid, &to_path);
            } else {
                knowledge_inserted(self.wid, &to_path);
            }
        }
        Ok(())
    }
}

/// Guest paths served to the in-guest FUSE forwarder reserve this top-level
/// directory for the workspace's local files; every other top-level segment is
/// a provider mount. A mount whose prefix is literally `files` would be shadowed
/// by the local tree — local wins.
const GUEST_LOCAL_DIR: &str = "files";

/// Map a guest path onto its local-subtree remainder, or `None` if it isn't
/// under `files/`. `/files` → `/`, `/files/a/b` → `/a/b`.
fn guest_local_rel(path: &str) -> Option<String> {
    let rest = path.trim_start_matches('/');
    if rest == GUEST_LOCAL_DIR {
        Some("/".to_string())
    } else {
        rest.strip_prefix(&format!("{GUEST_LOCAL_DIR}/"))
            .map(|sub| format!("/{sub}"))
    }
}

/// Serve the **unified** workspace tree to the in-guest FUSE forwarder: the
/// workspace's local files under a reserved `files/` subdirectory plus the
/// provider mounts as sibling virtual subdirectories (so the guest sees
/// `/mnt/workspace/files/…`, `/mnt/workspace/notion/…`, …).
///
/// Writable: local `files/` paths write to disk (through the inherent write
/// path, so `knowledge/` ingestion still fires), and provider paths delegate to
/// the [`Resource`], which decides by capability — S3 writes objects, a provider
/// whose `write_bytes` is `Unsupported` (e.g. Notion) stays read-only and
/// surfaces the error to the guest. The guest forwarder buffers FUSE writes and
/// sends the whole file on flush, so [`Self::write`] is a whole-file replace.
#[async_trait]
impl ForwardFs for WorkspaceFs {
    async fn readdir(&self, path: &str) -> anyhow::Result<Vec<FwdEntry>> {
        // Root: the reserved local dir plus each provider mount, all as dirs.
        if is_root_path(path) {
            let mut out = vec![FwdEntry {
                name: GUEST_LOCAL_DIR.to_string(),
                is_dir: true,
                size: 0,
                mtime: None,
            }];
            out.extend(self.mount_names().into_iter().map(|name| FwdEntry {
                name,
                is_dir: true,
                size: 0,
                mtime: None,
            }));
            return Ok(out);
        }

        // Local subtree: list disk directly. (Reusing `read_dir` would re-merge
        // the mount names, which belong only at the guest root.)
        if let Some(rel) = guest_local_rel(path) {
            let mut rd = tokio::fs::read_dir(self.resolve(&rel)).await?;
            let mut out = Vec::new();
            while let Some(entry) = rd.next_entry().await? {
                let (is_dir, size, mtime) = match entry.metadata().await {
                    Ok(m) => {
                        let s = Stat::from(m);
                        (s.is_dir(), s.len, s.modified.and_then(secs_since_epoch))
                    }
                    Err(_) => (false, 0, None),
                };
                out.push(FwdEntry {
                    name: String::from_utf8_lossy(&dir_entry_name(&entry)).into_owned(),
                    is_dir,
                    size,
                    mtime,
                });
            }
            return Ok(out);
        }

        // Provider mount: only a path that actually routes to a mount. A
        // top-level segment that is neither `files/` nor a mount prefix has no
        // node in the unified tree. The routed `read_dir` never merges.
        if self.route(path).is_none() {
            anyhow::bail!("no such directory: {path}");
        }
        let mut stream = self
            .read_dir(path, ReadDirMeta::None)
            .await
            .map_err(|e| anyhow::anyhow!("readdir {path}: {e:?}"))?;
        let mut out = Vec::new();
        while let Some(entry) = stream.next().await {
            let entry = entry.map_err(|e| anyhow::anyhow!("readdir {path}: {e:?}"))?;
            let (is_dir, size, mtime) = match entry.metadata() {
                Ok(st) => (st.is_dir(), st.len, st.modified.and_then(secs_since_epoch)),
                Err(_) => (false, 0, None),
            };
            out.push(FwdEntry {
                name: String::from_utf8_lossy(&entry.name()).into_owned(),
                is_dir,
                size,
                mtime,
            });
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> anyhow::Result<FwdStat> {
        if is_root_path(path) {
            return Ok(FwdStat {
                exists: true,
                is_dir: true,
                size: 0,
                mtime: None,
                atime: None,
                ctime: None,
            });
        }
        // Only two namespaces exist in the unified tree: local files under
        // `files/`, and provider mounts. A path in neither is ENOENT — it must
        // NOT fall through to `metadata`, which would resolve any bare name
        // against local disk (e.g. `/local.txt` → the file at the root).
        let target = match guest_local_rel(path) {
            Some(rel) => rel,
            None if self.route(path).is_some() => path.to_string(),
            None => return Ok(FwdStat::missing()),
        };
        Ok(match self.metadata(&target).await {
            Ok(st) => FwdStat {
                exists: true,
                is_dir: st.is_dir(),
                size: st.len,
                mtime: st.modified.and_then(secs_since_epoch),
                atime: st.accessed.and_then(secs_since_epoch),
                ctime: st.status_changed.and_then(secs_since_epoch),
            },
            // Any stat failure (missing path, provider error) reads as ENOENT,
            // matching the provider-only `Vfs` frontend.
            Err(_) => FwdStat::missing(),
        })
    }

    async fn read(
        &self,
        path: &str,
        offset: Option<u64>,
        size: Option<u64>,
    ) -> anyhow::Result<Vec<u8>> {
        let target = match guest_local_rel(path) {
            Some(rel) => rel,
            None if self.route(path).is_some() => path.to_string(),
            None => anyhow::bail!("no such file: {path}"),
        };
        let mut file = self
            .open(
                &target,
                OpenOptions {
                    read: true,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!("open {path}: {e:?}"))?;
        if let Some(off) = offset {
            file.seek(SeekFrom::Start(off))
                .await
                .map_err(|e| anyhow::anyhow!("seek {path}: {e:?}"))?;
        }
        // `read_bytes` may short-read (local `take` fills at most one buffer; a
        // mount pulls one range), so loop until the requested size is met or EOF.
        // A `None` size reads to EOF in bounded chunks.
        const CHUNK: usize = 256 * 1024;
        let mut out = Vec::new();
        loop {
            let want = match size {
                Some(s) => (s as usize).saturating_sub(out.len()),
                None => CHUNK,
            };
            if want == 0 {
                break;
            }
            let chunk = file
                .read_bytes(want.min(CHUNK))
                .await
                .map_err(|e| anyhow::anyhow!("read {path}: {e:?}"))?;
            if chunk.is_empty() {
                break;
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    async fn write(&self, path: &str, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        // Local file: whole-file replace (create+truncate), matching the guest
        // forwarder's flush semantics. Inherent path → `knowledge/` hooks fire.
        if let Some(rel) = guest_local_rel(path) {
            let mut f = self
                .open(
                    &rel,
                    OpenOptions {
                        write: true,
                        create: true,
                        truncate: true,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|e| anyhow::anyhow!("open {path}: {e:?}"))?;
            f.write_bytes(Bytes::from(data))
                .await
                .map_err(|e| anyhow::anyhow!("write {path}: {e:?}"))?;
            f.flush()
                .await
                .map_err(|e| anyhow::anyhow!("flush {path}: {e:?}"))?;
            return Ok(b"{\"ok\":true}".to_vec());
        }
        // Provider mount: delegate to the Resource; its capability decides
        // (S3 writes the object; an `Unsupported` provider surfaces the error).
        if let Some((res, vp)) = self.route(path) {
            res.write_bytes(&vp, data)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            return Ok(b"{\"ok\":true}".to_vec());
        }
        anyhow::bail!("no such file: {path}")
    }

    async fn unlink(&self, path: &str) -> anyhow::Result<()> {
        if let Some(rel) = guest_local_rel(path) {
            return self
                .remove_file(&rel)
                .await
                .map_err(|e| anyhow::anyhow!("unlink {path}: {e:?}"));
        }
        if let Some((res, vp)) = self.route(path) {
            return res.unlink(&vp).await.map_err(|e| anyhow::anyhow!("{e}"));
        }
        anyhow::bail!("no such file: {path}")
    }

    async fn mkdir(&self, path: &str) -> anyhow::Result<()> {
        if let Some(rel) = guest_local_rel(path) {
            return self
                .create_dir(&rel)
                .await
                .map_err(|e| anyhow::anyhow!("mkdir {path}: {e:?}"));
        }
        if let Some((res, vp)) = self.route(path) {
            return res.mkdir(&vp).await.map_err(|e| anyhow::anyhow!("{e}"));
        }
        anyhow::bail!("no such path: {path}")
    }

    async fn rmdir(&self, path: &str) -> anyhow::Result<()> {
        if let Some(rel) = guest_local_rel(path) {
            return self
                .remove_dir(&rel)
                .await
                .map_err(|e| anyhow::anyhow!("rmdir {path}: {e:?}"));
        }
        if let Some((res, vp)) = self.route(path) {
            return res.rmdir(&vp).await.map_err(|e| anyhow::anyhow!("{e}"));
        }
        anyhow::bail!("no such path: {path}")
    }

    async fn rename(&self, from: &str, to: &str) -> anyhow::Result<()> {
        match (guest_local_rel(from), guest_local_rel(to)) {
            // Both local — inherent `rename` wins over this trait method
            // (inherent methods take resolution precedence), so no recursion.
            (Some(a), Some(b)) => {
                return self
                    .rename(&a, &b)
                    .await
                    .map_err(|e| anyhow::anyhow!("rename {from} -> {to}: {e:?}"));
            }
            // Crossing the local/provider boundary isn't supported.
            (Some(_), None) | (None, Some(_)) => {
                anyhow::bail!("cross-boundary rename: {from} -> {to}");
            }
            (None, None) => {}
        }
        // Both provider: delegate to the Resource (same mount).
        match (self.route(from), self.route(to)) {
            (Some((res, from_vp)), Some((_, to_vp))) => res
                .rename(&from_vp, &to_vp)
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),
            _ => anyhow::bail!("no such path: {from} -> {to}"),
        }
    }
}

#[cfg(unix)]
fn dir_entry_name(entry: &tokio::fs::DirEntry) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    entry.file_name().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn dir_entry_name(entry: &tokio::fs::DirEntry) -> Vec<u8> {
    entry.file_name().to_string_lossy().as_bytes().to_vec()
}

/// True for the workspace root (`/` or empty), where mount names are surfaced.
fn is_root_path(rel_path: &str) -> bool {
    rel_path.trim_matches('/').is_empty()
}

/// Convert a provider directory entry into a workspace [`DirEntry`], carrying
/// the kind/size/mtime the provider reported.
fn dir_entry_from_vfs(e: crate::vfs::DirEntry) -> DirEntry {
    let stat = Stat {
        kind: node_kind_of(e.kind),
        len: e.size,
        modified: e.mtime,
        accessed: None,
        created: None,
        status_changed: None,
        executable: None,
    };
    DirEntry {
        name: e.name.into_bytes(),
        metadata: Ok(stat),
    }
}

/// A synthetic directory entry for a mount, listed at the workspace root.
fn mount_dir_entry(name: &str) -> DirEntry {
    DirEntry {
        name: name.as_bytes().to_vec(),
        metadata: Ok(Stat {
            kind: NodeKind::Dir,
            len: 0,
            modified: None,
            accessed: None,
            created: None,
            status_changed: None,
            executable: None,
        }),
    }
}

/// Build the `std::fs::OpenOptions` for opening a workspace file. On unix,
/// created files get private (`0o600`) mode, matching `dav_server`'s
/// non-public `LocalFs`. The async open converts this via
/// `tokio::fs::OpenOptions::from`.
fn open_options_std(options: &OpenOptions) -> std::fs::OpenOptions {
    let mut oo = std::fs::OpenOptions::new();
    oo.read(options.read)
        .write(options.write)
        .append(options.append)
        .truncate(options.truncate)
        .create(options.create)
        .create_new(options.create_new);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        oo.mode(0o600);
    }
    oo
}

/// Rename `from` to `to`. WebDAV permits renaming a directory over an existing
/// file, which `rename` rejects (`ENOTDIR`); detect that case and retry after
/// removing the destination file, mirroring `LocalFs`.
async fn rename_compat(from: &Path, to: &Path) -> io::Result<()> {
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
    use std::io::SeekFrom;
    use std::ops::Range;
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures_util::StreamExt;
    use uuid::Uuid;

    use super::{File, OpenOptions, ReadDirMeta, WorkspaceFs};
    use crate::state::FsError;
    use crate::vfs::{
        DirEntry as VfsDirEntry, FileKind, FileStat, Mount, Resource, VPath, Vfs, VfsError,
        VfsResult,
    };

    /// A tiny read-only provider holding one file, `/file.txt`.
    struct MockResource {
        content: Vec<u8>,
    }

    #[async_trait]
    impl Resource for MockResource {
        async fn read_bytes(
            &self,
            path: &VPath,
            range: Option<Range<u64>>,
        ) -> VfsResult<Vec<u8>> {
            if path.as_str() != "/file.txt" {
                return Err(VfsError::NotFound);
            }
            Ok(match range {
                Some(r) => {
                    let s = (r.start as usize).min(self.content.len());
                    let e = (r.end as usize).min(self.content.len());
                    self.content[s..e].to_vec()
                }
                None => self.content.clone(),
            })
        }

        async fn write_bytes(&self, _path: &VPath, _data: Vec<u8>) -> VfsResult<()> {
            Err(VfsError::Unsupported)
        }

        async fn readdir(&self, path: &VPath) -> VfsResult<Vec<VfsDirEntry>> {
            if path.is_root() {
                Ok(vec![VfsDirEntry {
                    name: "file.txt".to_string(),
                    kind: FileKind::File,
                    size: self.content.len() as u64,
                    mtime: None,
                    atime: None,
                    ctime: None,
                }])
            } else {
                Err(VfsError::NotFound)
            }
        }

        async fn stat(&self, path: &VPath) -> VfsResult<FileStat> {
            if path.is_root() {
                Ok(FileStat {
                    kind: FileKind::Dir,
                    ..Default::default()
                })
            } else if path.as_str() == "/file.txt" {
                Ok(FileStat {
                    kind: FileKind::File,
                    size: self.content.len() as u64,
                    ..Default::default()
                })
            } else {
                Err(VfsError::NotFound)
            }
        }
    }

    fn mounted_fs(root: std::path::PathBuf) -> WorkspaceFs {
        let resource: Arc<dyn Resource> = Arc::new(MockResource {
            content: b"hello world".to_vec(),
        });
        let vfs = Vfs::new(vec![Mount {
            prefix: "/mock".to_string(),
            resource,
        }])
        .unwrap();
        WorkspaceFs::new(root, Uuid::new_v4()).with_vfs(Some(Arc::new(vfs)))
    }

    async fn dir_names(fs: &WorkspaceFs, path: &str) -> Vec<String> {
        let mut stream = fs.read_dir(path, ReadDirMeta::None).await.unwrap();
        let mut names = Vec::new();
        while let Some(e) = stream.next().await {
            names.push(String::from_utf8(e.unwrap().name()).unwrap());
        }
        names.sort();
        names
    }

    #[tokio::test]
    async fn mount_metadata_read_and_seek() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        let st = fs.metadata("/mock/file.txt").await.unwrap();
        assert!(st.is_file());
        assert_eq!(st.len, 11);

        let mut f = fs
            .open(
                "/mock/file.txt",
                OpenOptions {
                    read: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(matches!(f, File::Mount(_)));
        let head = f.read_bytes(11).await.unwrap();
        assert_eq!(&head[..], b"hello world");

        // Seek from the start, then a bounded read.
        f.seek(SeekFrom::Start(6)).await.unwrap();
        let tail = f.read_bytes(5).await.unwrap();
        assert_eq!(&tail[..], b"world");
    }

    #[tokio::test]
    async fn mount_readdir_and_root_merge() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("local.txt"), b"x")
            .await
            .unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        // The mount lists provider entries.
        assert_eq!(dir_names(&fs, "/mock").await, vec!["file.txt"]);

        // The root lists local entries plus the mount name.
        let root = dir_names(&fs, "/").await;
        assert!(root.contains(&"local.txt".to_string()), "{root:?}");
        assert!(root.contains(&"mock".to_string()), "{root:?}");
    }

    #[tokio::test]
    async fn mount_is_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("local.txt"), b"x")
            .await
            .unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        let write_open = fs
            .open(
                "/mock/file.txt",
                OpenOptions {
                    write: true,
                    create: true,
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(write_open.err(), Some(FsError::Forbidden));
        assert_eq!(
            fs.create_dir("/mock/x").await.err(),
            Some(FsError::Forbidden)
        );
        assert_eq!(
            fs.remove_file("/mock/file.txt").await.err(),
            Some(FsError::Forbidden)
        );
        assert_eq!(
            fs.rename("/mock/a", "/mock/b").await.err(),
            Some(FsError::Forbidden)
        );
        assert_eq!(
            fs.copy("/local.txt", "/mock/b").await.err(),
            Some(FsError::Forbidden)
        );
    }

    #[tokio::test]
    async fn local_paths_unaffected_by_mounts() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("local.txt"), b"data")
            .await
            .unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        let st = fs.metadata("/local.txt").await.unwrap();
        assert!(st.is_file());
        assert_eq!(st.len, 4);
    }

    /// The `ForwardFs` view the in-guest FUSE forwarder consumes serves the
    /// unified tree: the workspace's local files under a reserved `files/`
    /// subdir plus the provider mounts as sibling virtual subdirectories, all
    /// through readdir/stat/read.
    #[tokio::test]
    async fn forward_fs_serves_unified_tree() {
        use crate::vfs::sandbox::ForwardFs;

        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("local.txt"), b"local-bytes")
            .await
            .unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        // Root lists the reserved local dir alongside the mount name — NOT the
        // local files directly.
        let mut root: Vec<(String, bool)> = ForwardFs::readdir(&fs, "/")
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        root.sort();
        assert_eq!(
            root,
            vec![("files".to_string(), true), ("mock".to_string(), true)]
        );

        // Local files live under `/files`.
        let local: Vec<String> = ForwardFs::readdir(&fs, "/files")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(local, vec!["local.txt".to_string()]);

        // The mount lists provider entries at its top-level prefix.
        let mount: Vec<(String, bool)> = ForwardFs::readdir(&fs, "/mock")
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        assert_eq!(mount, vec![("file.txt".to_string(), false)]);

        // Stat crosses both worlds; the reserved dir stats as a directory.
        assert!(ForwardFs::stat(&fs, "/files").await.unwrap().is_dir);
        let ls = ForwardFs::stat(&fs, "/files/local.txt").await.unwrap();
        assert!(ls.exists && !ls.is_dir && ls.size == 11);
        // Local files expose real mtime/atime/ctime from disk metadata.
        assert!(ls.mtime.is_some() && ls.atime.is_some() && ls.ctime.is_some());
        let ms = ForwardFs::stat(&fs, "/mock/file.txt").await.unwrap();
        assert!(ms.exists && !ms.is_dir && ms.size == 11);
        assert!(!ForwardFs::stat(&fs, "/nope").await.unwrap().exists);
        // A local file is no longer visible at the root.
        assert!(!ForwardFs::stat(&fs, "/local.txt").await.unwrap().exists);

        // Full and ranged reads of both a local file and a mounted object.
        assert_eq!(
            ForwardFs::read(&fs, "/files/local.txt", None, None)
                .await
                .unwrap(),
            b"local-bytes"
        );
        assert_eq!(
            ForwardFs::read(&fs, "/mock/file.txt", None, None)
                .await
                .unwrap(),
            b"hello world"
        );
        assert_eq!(
            ForwardFs::read(&fs, "/mock/file.txt", Some(6), Some(5))
                .await
                .unwrap(),
            b"world"
        );

        // Local writes succeed; a provider whose Resource lacks write support
        // (MockResource) stays read-only.
        assert!(
            ForwardFs::write(&fs, "/files/local.txt", b"x".to_vec())
                .await
                .is_ok()
        );
        assert!(
            ForwardFs::write(&fs, "/mock/file.txt", b"x".to_vec())
                .await
                .is_err()
        );
    }

    /// Local `files/` is read-write through the unified `ForwardFs` view —
    /// write / mkdir / rename / unlink land on disk; provider paths whose
    /// Resource lacks write support stay read-only.
    #[tokio::test]
    async fn forward_fs_local_write() {
        use crate::vfs::sandbox::ForwardFs;

        let tmp = tempfile::tempdir().unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        // Create then read back.
        ForwardFs::write(&fs, "/files/note.txt", b"hello".to_vec())
            .await
            .unwrap();
        assert_eq!(
            ForwardFs::read(&fs, "/files/note.txt", None, None).await.unwrap(),
            b"hello"
        );
        assert!(tmp.path().join("note.txt").exists());

        // Whole-file overwrite.
        ForwardFs::write(&fs, "/files/note.txt", b"world!".to_vec())
            .await
            .unwrap();
        assert_eq!(
            ForwardFs::read(&fs, "/files/note.txt", None, None).await.unwrap(),
            b"world!"
        );

        // mkdir + write inside it, then it lists.
        ForwardFs::mkdir(&fs, "/files/sub").await.unwrap();
        ForwardFs::write(&fs, "/files/sub/a.txt", b"x".to_vec())
            .await
            .unwrap();
        let sub: Vec<String> = ForwardFs::readdir(&fs, "/files/sub")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(sub, vec!["a.txt".to_string()]);

        // rename + unlink within local.
        ForwardFs::rename(&fs, "/files/note.txt", "/files/renamed.txt")
            .await
            .unwrap();
        assert!(ForwardFs::stat(&fs, "/files/renamed.txt").await.unwrap().exists);
        assert!(!ForwardFs::stat(&fs, "/files/note.txt").await.unwrap().exists);
        ForwardFs::unlink(&fs, "/files/renamed.txt").await.unwrap();
        assert!(!ForwardFs::stat(&fs, "/files/renamed.txt").await.unwrap().exists);

        // Provider stays read-only (MockResource → Unsupported), and crossing
        // the local/provider boundary is rejected.
        assert!(ForwardFs::write(&fs, "/mock/file.txt", b"x".to_vec()).await.is_err());
        assert!(ForwardFs::mkdir(&fs, "/mock/newdir").await.is_err());
        assert!(ForwardFs::rename(&fs, "/files/a", "/mock/b").await.is_err());
    }
}
