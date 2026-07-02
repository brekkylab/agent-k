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

use bytes::{Buf, Bytes, BytesMut};
use futures_util::{Stream, stream};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
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
    wid: Uuid,
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
    file: tokio::fs::File,
    buf: BytesMut,
    observer: Option<Observer>,
}

impl File {
    pub async fn metadata(&mut self) -> FsResult<std::fs::Metadata> {
        self.file.metadata().await.map_err(FsError::from)
    }

    pub async fn write_bytes(&mut self, buf: Bytes) -> FsResult<()> {
        if let Some(o) = self.observer.as_mut() {
            o.wrote = true;
        }
        self.file.write_all(&buf).await.map_err(FsError::from)
    }

    pub async fn write_buf(&mut self, mut buf: Box<dyn Buf + Send>) -> FsResult<()> {
        if let Some(o) = self.observer.as_mut() {
            o.wrote = true;
        }
        while buf.has_remaining() {
            let n = self.file.write(buf.chunk()).await.map_err(FsError::from)?;
            buf.advance(n);
        }
        Ok(())
    }

    pub async fn read_bytes(&mut self, count: usize) -> FsResult<Bytes> {
        // Reuse `self.buf`'s allocation across reads; cap the read at `count`
        // and hand back exactly the bytes filled (an empty `Bytes` at EOF).
        let mut buf = std::mem::take(&mut self.buf);
        buf.reserve(count);
        let res = (&mut self.file).take(count as u64).read_buf(&mut buf).await;
        self.buf = buf;
        res.map_err(FsError::from)?;
        Ok(self.buf.split().freeze())
    }

    pub async fn seek(&mut self, pos: SeekFrom) -> FsResult<u64> {
        self.file.seek(pos).await.map_err(FsError::from)
    }

    pub async fn flush(&mut self) -> FsResult<()> {
        self.file.flush().await?;
        if let Some(o) = self.observer.as_mut()
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
}

/// A filesystem handle scoped to a single workspace. Cheap to clone
/// (just an owned root path and workspace id); `Send + Sync + 'static`, so the
/// WebDAV layer can hold one for the lifetime of a request.
#[derive(Clone)]
pub struct WorkspaceFs {
    root: PathBuf,
    wid: Uuid,
}

impl WorkspaceFs {
    pub(super) fn new(root: PathBuf, wid: Uuid) -> Self {
        Self { root, wid }
    }

    /// Absolute on-disk path of `rel_path` (a workspace-relative path such as
    /// `/knowledge/foo.txt`) inside this workspace.
    fn resolve(&self, rel_path: &str) -> PathBuf {
        self.root.join(rel_path.trim_start_matches('/'))
    }

    pub async fn metadata(&self, rel_path: &str) -> FsResult<std::fs::Metadata> {
        tokio::fs::metadata(self.resolve(rel_path))
            .await
            .map_err(FsError::from)
    }

    pub async fn symlink_metadata(&self, rel_path: &str) -> FsResult<std::fs::Metadata> {
        tokio::fs::symlink_metadata(self.resolve(rel_path))
            .await
            .map_err(FsError::from)
    }

    pub async fn read_dir(&self, rel_path: &str, meta: ReadDirMeta) -> FsResult<DirStream> {
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
                        metadata: md.map_err(FsError::from),
                    }));
                }
                Ok(None) => break,
                Err(e) => {
                    out.push(Err(FsError::from(e)));
                    break;
                }
            }
        }
        Ok(Box::pin(stream::iter(out)))
    }

    pub async fn open(&self, rel_path: &str, options: OpenOptions) -> FsResult<File> {
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
        Ok(File {
            file,
            buf: BytesMut::new(),
            observer,
        })
    }

    pub async fn create_dir(&self, rel_path: &str) -> FsResult<()> {
        let path = self.resolve(rel_path);
        let mut builder = tokio::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            builder.mode(0o700);
        }
        builder.create(&path).await.map_err(FsError::from)
    }

    pub async fn remove_dir(&self, rel_path: &str) -> FsResult<()> {
        tokio::fs::remove_dir(self.resolve(rel_path))
            .await
            .map_err(FsError::from)
    }

    pub async fn remove_file(&self, rel_path: &str) -> FsResult<()> {
        let path = self.resolve(rel_path);
        tokio::fs::remove_file(&path).await.map_err(FsError::from)?;
        if is_knowledge(rel_path) {
            knowledge_removed(self.wid, &path);
        }
        Ok(())
    }

    pub async fn rename(&self, from: &str, to: &str) -> FsResult<()> {
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

#[cfg(unix)]
fn dir_entry_name(entry: &tokio::fs::DirEntry) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    entry.file_name().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn dir_entry_name(entry: &tokio::fs::DirEntry) -> Vec<u8> {
    entry.file_name().to_string_lossy().as_bytes().to_vec()
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
