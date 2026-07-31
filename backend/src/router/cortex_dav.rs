//! Adapts a cortex [`Workspace`](cortex::Workspace) onto `dav_server`'s
//! [`DavFileSystem`], so agent-k serves the *same* unified tree over WebDAV that
//! it hands the sandbox — one `Workspace`, two frontends.
//!
//! cortex's `Mountable` API is synchronous (it is driven by FUSE/virtio-fs
//! worker threads elsewhere), while `DavFileSystem` is async. Each op therefore
//! runs on [`spawn_blocking`](tokio::task::spawn_blocking); the workspace is
//! shared as an `Arc` (cortex provides `impl Mountable for Arc<T>`), and open
//! files hold an `Arc<dyn FileHandle>` whose positioned I/O is `&self`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use cortex::{CortexError, DirentKind, FileHandle, Mountable, OpenOptions as CxOpenOptions,
    Stat, Workspace};
use dav_server::davpath::DavPath;
use dav_server::fs::{
    DavDirEntry, DavFile, DavFileSystem, DavMetaData, FsError, FsFuture, FsResult, FsStream,
    OpenOptions, ReadDirMeta,
};

/// A cortex workspace served over WebDAV.
#[derive(Clone)]
pub struct CortexDavFs {
    ws: Arc<Workspace>,
}

impl CortexDavFs {
    pub fn new(ws: Arc<Workspace>) -> Self {
        Self { ws }
    }
}

/// Workspace-relative path for a `DavPath`. `as_rel_ospath` drops the leading
/// slash; cortex treats a request path as relative to the workspace root.
fn rel(path: &DavPath) -> PathBuf {
    PathBuf::from(path.as_rel_ospath())
}

/// Map a cortex error onto the WebDAV `FsError`.
fn to_dav(e: CortexError) -> FsError {
    match e {
        CortexError::NotFound => FsError::NotFound,
        CortexError::AlreadyExists => FsError::Exists,
        CortexError::ReadOnly
        | CortexError::Unsupported
        | CortexError::PermissionDenied => FsError::Forbidden,
        _ => FsError::GeneralFailure,
    }
}

impl DavFileSystem for CortexDavFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        let ws = self.ws.clone();
        let p = rel(path);
        Box::pin(async move {
            let opts = CxOpenOptions {
                read: options.read,
                write: options.write,
                append: options.append,
                truncate: options.truncate,
                create: options.create,
                create_new: options.create_new,
            };
            let (handle, stat) = tokio::task::spawn_blocking(move || ws.open(&p, opts))
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(to_dav)?;
            Ok(Box::new(CortexDavFile {
                handle: Arc::from(handle),
                cursor: AtomicU64::new(0),
                stat,
            }) as Box<dyn DavFile>)
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        _meta: ReadDirMeta,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        let ws = self.ws.clone();
        let p = rel(path);
        Box::pin(async move {
            let entries = {
                let ws = ws.clone();
                let p = p.clone();
                tokio::task::spawn_blocking(move || ws.list(&p))
                    .await
                    .map_err(|_| FsError::GeneralFailure)?
                    .map_err(to_dav)?
            };
            let items: Vec<Box<dyn DavDirEntry>> = entries
                .into_iter()
                .map(|e| {
                    Box::new(CortexDirEntry {
                        ws: ws.clone(),
                        path: p.join(&e.name),
                        name: e.name,
                        stat: e.stat,
                    }) as Box<dyn DavDirEntry>
                })
                .collect();
            Ok(Box::pin(futures_util::stream::iter(items.into_iter().map(Ok)))
                as FsStream<Box<dyn DavDirEntry>>)
        })
    }

    fn metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        let ws = self.ws.clone();
        let p = rel(path);
        Box::pin(async move {
            let stat = tokio::task::spawn_blocking(move || ws.stat(&p))
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(to_dav)?;
            Ok(Box::new(CortexMeta(stat)) as Box<dyn DavMetaData>)
        })
    }

    fn create_dir<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        let ws = self.ws.clone();
        let p = rel(path);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || ws.mkdir(&p))
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(to_dav)
        })
    }

    fn remove_dir<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        let ws = self.ws.clone();
        let p = rel(path);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || ws.rmdir(&p))
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(to_dav)
        })
    }

    fn remove_file<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        let ws = self.ws.clone();
        let p = rel(path);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || ws.unlink(&p))
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(to_dav)
        })
    }

    fn rename<'a>(&'a self, from: &'a DavPath, to: &'a DavPath) -> FsFuture<'a, ()> {
        let ws = self.ws.clone();
        let (f, t) = (rel(from), rel(to));
        Box::pin(async move {
            tokio::task::spawn_blocking(move || ws.rename(&f, &t))
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(to_dav)
        })
    }

    // `copy` has no cortex primitive; WebDAV COPY is optional and left for later
    // (falls back to the trait's `NotImplemented`).
}

/// An open workspace file over WebDAV. cortex handles are offset-addressed
/// (`&self` positioned I/O), so this tracks the WebDAV cursor separately.
struct CortexDavFile {
    handle: Arc<dyn FileHandle>,
    cursor: AtomicU64,
    stat: Stat,
}

impl std::fmt::Debug for CortexDavFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CortexDavFile").finish_non_exhaustive()
    }
}

impl DavFile for CortexDavFile {
    fn metadata(&mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        let stat = self.stat.clone();
        Box::pin(async move { Ok(Box::new(CortexMeta(stat)) as Box<dyn DavMetaData>) })
    }

    fn write_buf(&mut self, mut buf: Box<dyn bytes::Buf + Send>) -> FsFuture<'_, ()> {
        Box::pin(async move {
            let bytes = buf.copy_to_bytes(buf.remaining());
            self.write_bytes(bytes).await
        })
    }

    fn write_bytes(&mut self, buf: bytes::Bytes) -> FsFuture<'_, ()> {
        let handle = self.handle.clone();
        let offset = self.cursor.load(Ordering::Relaxed);
        Box::pin(async move {
            let n = buf.len() as u64;
            tokio::task::spawn_blocking(move || handle.write_all_at(&buf, offset))
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(|_| FsError::GeneralFailure)?;
            self.cursor.fetch_add(n, Ordering::Relaxed);
            if offset + n > self.stat.size {
                self.stat.size = offset + n;
            }
            Ok(())
        })
    }

    fn read_bytes(&mut self, count: usize) -> FsFuture<'_, bytes::Bytes> {
        let handle = self.handle.clone();
        let offset = self.cursor.load(Ordering::Relaxed);
        let remaining = self.stat.size.saturating_sub(offset);
        let n = (count as u64).min(remaining) as usize;
        Box::pin(async move {
            if n == 0 {
                return Ok(bytes::Bytes::new());
            }
            let data = tokio::task::spawn_blocking(move || {
                let mut buf = vec![0u8; n];
                handle.read_exact_at(&mut buf, offset).map(|_| buf)
            })
            .await
            .map_err(|_| FsError::GeneralFailure)?
            .map_err(|_| FsError::GeneralFailure)?;
            self.cursor.fetch_add(n as u64, Ordering::Relaxed);
            Ok(bytes::Bytes::from(data))
        })
    }

    fn seek(&mut self, pos: std::io::SeekFrom) -> FsFuture<'_, u64> {
        use std::io::SeekFrom;
        let new = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::End(d) => (self.stat.size as i64 + d).max(0) as u64,
            SeekFrom::Current(d) => {
                (self.cursor.load(Ordering::Relaxed) as i64 + d).max(0) as u64
            }
        };
        self.cursor.store(new, Ordering::Relaxed);
        Box::pin(async move { Ok(new) })
    }

    fn flush(&mut self) -> FsFuture<'_, ()> {
        let handle = self.handle.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || handle.flush())
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(to_dav)
        })
    }
}

/// A cortex [`Stat`] as WebDAV metadata.
#[derive(Clone, Debug)]
struct CortexMeta(Stat);

impl DavMetaData for CortexMeta {
    fn len(&self) -> u64 {
        self.0.size
    }

    fn modified(&self) -> FsResult<SystemTime> {
        Ok(self.0.mtime.unwrap_or(UNIX_EPOCH))
    }

    fn is_dir(&self) -> bool {
        self.0.kind == DirentKind::Dir
    }

    fn is_file(&self) -> bool {
        self.0.kind == DirentKind::File
    }

    fn accessed(&self) -> FsResult<SystemTime> {
        self.0.atime.ok_or(FsError::NotImplemented)
    }

    fn created(&self) -> FsResult<SystemTime> {
        self.0.created.ok_or(FsError::NotImplemented)
    }

    fn status_changed(&self) -> FsResult<SystemTime> {
        self.0.ctime.ok_or(FsError::NotImplemented)
    }
}

/// A directory entry. Carries the workspace + child path so `metadata` can stat
/// lazily when the listing did not include it (a local passthrough), and use the
/// free metadata otherwise (an object store / provider that returns it in-list).
struct CortexDirEntry {
    ws: Arc<Workspace>,
    path: PathBuf,
    name: String,
    stat: Option<Stat>,
}

impl DavDirEntry for CortexDirEntry {
    fn name(&self) -> Vec<u8> {
        self.name.as_bytes().to_vec()
    }

    fn metadata(&self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        if let Some(stat) = &self.stat {
            let stat = stat.clone();
            return Box::pin(async move { Ok(Box::new(CortexMeta(stat)) as Box<dyn DavMetaData>) });
        }
        let ws = self.ws.clone();
        let p = self.path.clone();
        Box::pin(async move {
            let stat = tokio::task::spawn_blocking(move || ws.stat(&p))
                .await
                .map_err(|_| FsError::GeneralFailure)?
                .map_err(to_dav)?;
            Ok(Box::new(CortexMeta(stat)) as Box<dyn DavMetaData>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex::{VolumeSpec, WorkspaceSpec};
    use futures_util::StreamExt;
    use std::fs;

    /// Drive the adapter's `DavFileSystem` surface directly (no HTTP): stat, list,
    /// read, and a create+write that lands on the backing dir — proving a cortex
    /// workspace is served correctly over the WebDAV contract.
    #[tokio::test]
    async fn serves_a_cortex_workspace() {
        let dir = std::env::temp_dir().join(format!("cortex-dav-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("files")).unwrap();
        fs::write(dir.join("files/hello.txt"), b"world").unwrap();

        let spec = WorkspaceSpec::default()
            .mount("files", VolumeSpec::Local { host: dir.join("files") });
        let davfs = CortexDavFs::new(Arc::new(Workspace::from_spec(&spec).unwrap()));

        // metadata + read of an existing file.
        let hello = DavPath::new("/files/hello.txt").unwrap();
        let meta = davfs.metadata(&hello).await.unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.len(), 5);
        let mut f = davfs
            .open(&hello, OpenOptions { read: true, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(&f.read_bytes(5).await.unwrap()[..], b"world");

        // create + write; it lands on the backing dir.
        let new = DavPath::new("/files/new.txt").unwrap();
        let mut nf = davfs
            .open(&new, OpenOptions { write: true, create: true, ..Default::default() })
            .await
            .unwrap();
        nf.write_bytes(bytes::Bytes::from_static(b"via-dav")).await.unwrap();
        nf.flush().await.unwrap();
        assert_eq!(fs::read(dir.join("files/new.txt")).unwrap(), b"via-dav");

        // list shows the mount's children.
        let mut names: Vec<String> = davfs
            .read_dir(&DavPath::new("/files").unwrap(), ReadDirMeta::Data)
            .await
            .unwrap()
            .map(|e| String::from_utf8_lossy(&e.unwrap().name()).into_owned())
            .collect()
            .await;
        names.sort();
        assert_eq!(names, vec!["hello.txt", "new.txt"]);

        let _ = fs::remove_dir_all(&dir);
    }
}
