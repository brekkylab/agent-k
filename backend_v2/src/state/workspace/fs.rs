//! Protocol-agnostic filesystem primitives for a project's workspace.
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

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;

use bytes::{Buf, Bytes, BytesMut};
use futures_util::{Stream, stream};
use uuid::Uuid;

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

/// Run blocking filesystem work without stalling the async runtime, matching
/// the strategy `dav_server`'s `LocalFs` uses: `block_in_place` on a
/// multi-thread runtime, `spawn_blocking` otherwise.
async fn blocking<F, R>(func: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    match tokio::runtime::Handle::current().runtime_flavor() {
        tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(func),
        _ => tokio::task::spawn_blocking(func).await.unwrap(),
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
fn knowledge_inserted(pid: Uuid, path: &Path) {
    tracing::info!("insert_knowledge (project={pid}, path={})", path.display());
}

/// An existing file was overwritten in place.
fn knowledge_updated(pid: Uuid, path: &Path) {
    tracing::info!("update_knowledge (project={pid}, path={})", path.display());
}

/// A file left `knowledge/` (a delete, or a move whose source was under it).
/// The file is already gone from disk; `path` is where it lived.
fn knowledge_removed(pid: Uuid, path: &Path) {
    tracing::info!("remove_knowledge (project={pid}, path={})", path.display());
}

/// One entry yielded by [`WorkspaceFs::read_dir`]. Metadata is captured eagerly
/// at listing time.
pub struct DirEntry {
    name: Vec<u8>,
    metadata: FsResult<std::fs::Metadata>,
}

impl DirEntry {
    /// Raw filename bytes (no path).
    pub fn name(&self) -> Vec<u8> {
        self.name.clone()
    }

    /// Metadata captured when the directory was listed.
    pub fn metadata(&self) -> FsResult<std::fs::Metadata> {
        self.metadata.clone()
    }
}

/// A stream of directory entries.
pub type DirStream = Pin<Box<dyn Stream<Item = FsResult<DirEntry>> + Send>>;

/// Tracks an in-flight write so its completion (`flush` after a `write_*`) can
/// be reported to the workspace's side-processing.
struct Observer {
    pid: Uuid,
    /// Absolute on-disk path of the file, for the knowledge handlers.
    path: PathBuf,
    /// Whether the file already existed when opened — distinguishes an insert
    /// from an overwrite at flush time.
    existed: bool,
    wrote: bool,
}

/// An open workspace file. Readable / writable / seekable, like
/// [`std::fs::File`], and — for write opens — reports a completed write to the
/// workspace once `flush` follows at least one `write_*`.
pub struct File {
    file: Option<std::fs::File>,
    buf: BytesMut,
    observer: Option<Observer>,
}

impl File {
    pub async fn metadata(&mut self) -> FsResult<std::fs::Metadata> {
        let file = self.file.take().unwrap();
        let (meta, file) = blocking(move || (file.metadata(), file)).await;
        self.file = Some(file);
        meta.map_err(FsError::from)
    }

    pub async fn write_bytes(&mut self, buf: Bytes) -> FsResult<()> {
        if let Some(o) = self.observer.as_mut() {
            o.wrote = true;
        }
        let mut file = self.file.take().unwrap();
        let (res, file) = blocking(move || (file.write_all(&buf), file)).await;
        self.file = Some(file);
        res.map_err(FsError::from)
    }

    pub async fn write_buf(&mut self, mut buf: Box<dyn Buf + Send>) -> FsResult<()> {
        if let Some(o) = self.observer.as_mut() {
            o.wrote = true;
        }
        let mut file = self.file.take().unwrap();
        let (res, file) = blocking(move || {
            while buf.remaining() > 0 {
                let n = match file.write(buf.chunk()) {
                    Ok(n) => n,
                    Err(e) => return (Err(e), file),
                };
                buf.advance(n);
            }
            (Ok(()), file)
        })
        .await;
        self.file = Some(file);
        res.map_err(FsError::from)
    }

    pub async fn read_bytes(&mut self, count: usize) -> FsResult<Bytes> {
        let mut file = self.file.take().unwrap();
        let mut buf = std::mem::take(&mut self.buf);
        let (res, file, buf) = blocking(move || {
            buf.reserve(count);
            let res = unsafe {
                buf.set_len(count);
                file.read(&mut buf).map(|n| {
                    buf.set_len(n);
                    buf.split().freeze()
                })
            };
            (res, file, buf)
        })
        .await;
        self.file = Some(file);
        self.buf = buf;
        res.map_err(FsError::from)
    }

    pub async fn seek(&mut self, pos: SeekFrom) -> FsResult<u64> {
        let mut file = self.file.take().unwrap();
        let (res, file) = blocking(move || (file.seek(pos), file)).await;
        self.file = Some(file);
        res.map_err(FsError::from)
    }

    pub async fn flush(&mut self) -> FsResult<()> {
        let mut file = self.file.take().unwrap();
        let (res, file) = blocking(move || (file.flush(), file)).await;
        self.file = Some(file);
        res?;
        if let Some(o) = self.observer.as_mut()
            && o.wrote
        {
            // Clear first so a second flush on the same handle won't re-report.
            o.wrote = false;
            if o.existed {
                knowledge_updated(o.pid, &o.path);
            } else {
                knowledge_inserted(o.pid, &o.path);
            }
        }
        Ok(())
    }
}

/// A filesystem handle scoped to a single project's workspace. Cheap to clone
/// (just an owned root path and project id); `Send + Sync + 'static`, so the
/// WebDAV layer can hold one for the lifetime of a request.
#[derive(Clone)]
pub struct WorkspaceFs {
    root: PathBuf,
    pid: Uuid,
}

impl WorkspaceFs {
    pub(super) fn new(root: PathBuf, pid: Uuid) -> Self {
        Self { root, pid }
    }

    /// Absolute on-disk path of `rel_path` (a workspace-relative path such as
    /// `/knowledge/foo.txt`) inside this project's workspace.
    fn resolve(&self, rel_path: &str) -> PathBuf {
        self.root.join(rel_path.trim_start_matches('/'))
    }

    pub async fn metadata(&self, rel_path: &str) -> FsResult<std::fs::Metadata> {
        let path = self.resolve(rel_path);
        blocking(move || std::fs::metadata(&path).map_err(FsError::from)).await
    }

    pub async fn symlink_metadata(&self, rel_path: &str) -> FsResult<std::fs::Metadata> {
        let path = self.resolve(rel_path);
        blocking(move || std::fs::symlink_metadata(&path).map_err(FsError::from)).await
    }

    pub async fn read_dir(&self, rel_path: &str, meta: ReadDirMeta) -> FsResult<DirStream> {
        let path = self.resolve(rel_path);
        let entries = blocking(move || -> io::Result<Vec<FsResult<DirEntry>>> {
            let mut out = Vec::new();
            for entry in std::fs::read_dir(&path)? {
                match entry {
                    Ok(entry) => {
                        let md = match meta {
                            ReadDirMeta::Data => std::fs::metadata(entry.path()),
                            ReadDirMeta::DataSymlink | ReadDirMeta::None => entry.metadata(),
                        };
                        out.push(Ok(DirEntry {
                            name: dir_entry_name(&entry),
                            metadata: md.map_err(FsError::from),
                        }));
                    }
                    Err(e) => {
                        out.push(Err(FsError::from(e)));
                        break;
                    }
                }
            }
            Ok(out)
        })
        .await?;
        Ok(Box::pin(stream::iter(entries)))
    }

    pub async fn open(&self, rel_path: &str, options: OpenOptions) -> FsResult<File> {
        let is_write = options.write || options.append || options.create || options.create_new;
        let path = self.resolve(rel_path);
        // Probe before opening: a create/truncating open would make the file
        // exist (or empty) regardless, so capture prior existence here to tell
        // an insert from an overwrite at flush time.
        let existed = if is_write {
            let probe = path.clone();
            blocking(move || std::fs::metadata(&probe).is_ok()).await
        } else {
            false
        };
        let file = {
            let path = path.clone();
            blocking(move || open_std(&path, &options)).await?
        };
        // Only knowledge writes need observing; the flush hook then reports an
        // insert or update without re-classifying.
        let observer = (is_write && is_knowledge(rel_path)).then_some(Observer {
            pid: self.pid,
            path,
            existed,
            wrote: false,
        });
        Ok(File {
            file: Some(file),
            buf: BytesMut::new(),
            observer,
        })
    }

    pub async fn create_dir(&self, rel_path: &str) -> FsResult<()> {
        let path = self.resolve(rel_path);
        blocking(move || create_dir_std(&path).map_err(FsError::from)).await
    }

    pub async fn remove_dir(&self, rel_path: &str) -> FsResult<()> {
        let path = self.resolve(rel_path);
        blocking(move || std::fs::remove_dir(&path).map_err(FsError::from)).await
    }

    pub async fn remove_file(&self, rel_path: &str) -> FsResult<()> {
        let path = self.resolve(rel_path);
        {
            let path = path.clone();
            blocking(move || std::fs::remove_file(&path).map_err(FsError::from)).await?;
        }
        if is_knowledge(rel_path) {
            knowledge_removed(self.pid, &path);
        }
        Ok(())
    }

    pub async fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        // Probe the destination before the move so we can tell whether it
        // landed on a fresh path (insert) or replaced one (update).
        let to_existed = self.metadata(to).await.is_ok();
        let from_path = self.resolve(from);
        let to_path = self.resolve(to);
        {
            let (src, dst) = (from_path.clone(), to_path.clone());
            blocking(move || rename_std(&src, &dst).map_err(FsError::from)).await?;
        }
        // The source path left `knowledge/`; the destination arrived in it.
        if is_knowledge(from) {
            knowledge_removed(self.pid, &from_path);
        }
        if is_knowledge(to) {
            if to_existed {
                knowledge_updated(self.pid, &to_path);
            } else {
                knowledge_inserted(self.pid, &to_path);
            }
        }
        Ok(())
    }

    pub async fn copy(&self, from: &str, to: &str) -> FsResult<()> {
        let to_existed = self.metadata(to).await.is_ok();
        let from_path = self.resolve(from);
        let to_path = self.resolve(to);
        {
            let (src, dst) = (from_path, to_path.clone());
            blocking(move || std::fs::copy(&src, &dst).map_err(FsError::from)).await?;
        }
        if is_knowledge(to) {
            if to_existed {
                knowledge_updated(self.pid, &to_path);
            } else {
                knowledge_inserted(self.pid, &to_path);
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn dir_entry_name(entry: &std::fs::DirEntry) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    entry.file_name().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn dir_entry_name(entry: &std::fs::DirEntry) -> Vec<u8> {
    entry.file_name().to_string_lossy().as_bytes().to_vec()
}

/// Open a file at `path`. On unix, created files get private (`0o600`) mode,
/// matching `dav_server`'s non-public `LocalFs`.
fn open_std(path: &Path, options: &OpenOptions) -> io::Result<std::fs::File> {
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
    oo.open(path)
}

/// Create a directory at `path`. On unix it is private (`0o700`).
fn create_dir_std(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Rename `from` to `to`. WebDAV permits renaming a directory over an existing
/// file, which `std::fs::rename` rejects (`ENOTDIR`); detect that case and
/// retry after removing the destination file, mirroring `LocalFs`.
fn rename_std(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) => {
            if from.is_dir() && to.is_file() {
                let _ = std::fs::remove_file(to);
                std::fs::rename(from, to)
            } else {
                Err(e)
            }
        }
    }
}
