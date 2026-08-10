//! Protocol-agnostic filesystem primitives for a workspace.
//!
//! [`WorkspaceFs`] is a thin router over its own mount table: the
//! workspace's local files (the [`LOCAL_MOUNT`](crate::vfs::LOCAL_MOUNT) `/files`
//! mount, a `LocalResource`) and its external-provider mounts (`/notion`, …) are
//! all [`Resource`]s. Every path operation is `route → delegate`; there is no
//! separate local-disk branch.
//!
//! These types ([`WorkspaceFs`], [`File`], [`DirEntry`], …) carry **no**
//! dependency on `dav_server`: they speak workspace-relative path strings, owned
//! byte buffers, and `std`/`tokio` types. The WebDAV protocol layer adapts them
//! onto `dav_server`'s traits; the in-guest FUSE forwarder consumes the same
//! tree through [`ForwardFs`].
//!
//! Every mutating operation fires the injected [`FsHook`] (if any) so a host can
//! observe changes (e.g. `knowledge/` ingestion); the classification is the
//! host's, not this crate's.

use std::io::SeekFrom;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use bytes::{Buf, Bytes};
use futures_util::{Stream, StreamExt as _, stream};

use crate::hook::{FsEvent, FsHook};
use crate::vfs::sandbox::{ForwardFs, FwdEntry, FwdStat, secs_since_epoch};
use crate::vfs::{
    FileKind, FileStat, FsConfig, Mount, MountPath, Resource, ResourceError, build_mounts,
};

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

/// Map a provider [`ResourceError`] onto the workspace [`FsError`]. The provider
/// classifies the failure itself (typed, not by message): `NotFound` → 404, an
/// unsupported op → `NotImplemented`, any backend failure → a generic error.
impl From<ResourceError> for FsError {
    fn from(e: ResourceError) -> Self {
        match e {
            ResourceError::NotFound => FsError::NotFound,
            ResourceError::Unsupported => FsError::NotImplemented,
            ResourceError::Backend(_) => FsError::GeneralFailure,
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

/// The kind of a filesystem node. Symlinks are not modelled — a symlink resolves
/// to its target — so only [`File`](NodeKind::File) / [`Dir`](NodeKind::Dir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
}

/// Protocol-agnostic file metadata: the fields the WebDAV layer reads, decoupled
/// from [`std::fs::Metadata`] so a node can equally be a local file or an
/// external provider object. Timestamps are `Option` because non-local sources
/// (S3, Notion, …) don't report them all; the WebDAV adapter maps a `None` onto
/// the corresponding `NotImplemented`. `executable` is always `None` now (the
/// unified path doesn't report the unix mode).
#[derive(Debug, Clone)]
pub struct Stat {
    pub kind: NodeKind,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub status_changed: Option<SystemTime>,
    pub executable: Option<bool>,
    /// What the node's subject is, when the provider knows better than the
    /// filename (a Google Doc is served as `report.gdoc.json`, so the extension
    /// only ever says `application/json`). `None` = go by the name.
    pub content_type: Option<String>,
}

impl Stat {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, NodeKind::Dir)
    }

    pub fn is_file(&self) -> bool {
        matches!(self.kind, NodeKind::File)
    }

    /// Symlinks are resolved to their target, so a `Stat` never describes one.
    pub fn is_symlink(&self) -> bool {
        false
    }
}

/// Map a VFS [`FileKind`] onto the local [`NodeKind`].
fn node_kind_of(kind: FileKind) -> NodeKind {
    match kind {
        FileKind::Dir => NodeKind::Dir,
        FileKind::File => NodeKind::File,
    }
}

/// Build a [`Stat`] from a [`Resource`]'s [`FileStat`]. `executable` has no
/// provider representation, so it stays `None`.
fn stat_from_vfs(fs: FileStat) -> Stat {
    Stat {
        kind: node_kind_of(fs.kind),
        len: fs.size,
        modified: fs.mtime,
        accessed: fs.atime,
        created: fs.created,
        status_changed: fs.ctime,
        executable: None,
        content_type: fs.content_type,
    }
}

/// The synthetic `Stat` for a directory node with no timestamps — the virtual
/// root and each mount name surfaced there.
fn dir_stat() -> Stat {
    Stat {
        kind: NodeKind::Dir,
        len: 0,
        modified: None,
        accessed: None,
        created: None,
        status_changed: None,
        executable: None,
        content_type: None,
    }
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

/// Tracks an in-flight write so its completion (`flush` after a `write_*`) can be
/// reported to the injected [`FsHook`] exactly once, with the create-vs-modify
/// distinction. Attached only on a write open when a hook is present.
struct Observer {
    /// Workspace-relative path of the file, for the change event.
    rel: String,
    /// Whether the target already existed when opened — distinguishes a create
    /// from an overwrite at flush time.
    existed: bool,
    wrote: bool,
    /// The workspace's hook, cloned at open time so `flush` can report the
    /// completed write without reaching back into the [`WorkspaceFs`].
    hook: Option<Arc<dyn FsHook>>,
}

/// An open workspace file: a cursor over the owning [`Resource`]. Reads pull a
/// byte range from the resource at the current offset; writes accumulate in
/// `wbuf` and are flushed as one whole-object `write_bytes`. The same handle
/// serves a local file or a mounted provider object — the resource decides
/// (a provider whose `write_bytes` is `Unsupported`, e.g. Notion, fails at flush).
pub struct File {
    resource: Arc<dyn Resource>,
    path: MountPath,
    offset: u64,
    /// Provider stat captured when the file opened; pins the version so every
    /// chunk of one read comes from a single snapshot (no per-chunk stat).
    stat: FileStat,
    /// Whether this handle was opened for writing (gates `write_*`/`flush`).
    write: bool,
    /// Accumulated write buffer, flushed as the whole object.
    wbuf: Vec<u8>,
    observer: Option<Observer>,
}

impl File {
    /// Metadata of the open file.
    ///
    /// A provider that cannot size a rendered file reports an upper bound (see
    /// [`FileStat::size_is_estimate`]), which is right for a listing — `ls -l`
    /// must not render a folder's worth of documents. It is wrong for whoever
    /// opened the file: a WebDAV `GET` fills `Content-Length` from here, and an
    /// over-estimate would advertise more bytes than the body carries. So on an
    /// open handle the estimate is resolved by producing the content once, which
    /// the caller is about to read anyway (and the metadata cache keeps).
    pub async fn metadata(&mut self) -> FsResult<Stat> {
        let st = self
            .resource
            .stat(&self.path)
            .await
            .map_err(FsError::from)?;
        if !st.size_is_estimate {
            return Ok(stat_from_vfs(st));
        }
        let len = self
            .resource
            .read_bytes(&self.path, None)
            .await
            .map_err(FsError::from)?
            .len() as u64;
        Ok(Stat {
            len,
            ..stat_from_vfs(st)
        })
    }

    pub async fn write_bytes(&mut self, buf: Bytes) -> FsResult<()> {
        if !self.write {
            return Err(FsError::Forbidden);
        }
        self.wbuf.extend_from_slice(&buf);
        if let Some(o) = self.observer.as_mut() {
            o.wrote = true;
        }
        Ok(())
    }

    pub async fn write_buf(&mut self, mut buf: Box<dyn Buf + Send>) -> FsResult<()> {
        if !self.write {
            return Err(FsError::Forbidden);
        }
        while buf.has_remaining() {
            let chunk = buf.chunk();
            self.wbuf.extend_from_slice(chunk);
            let n = chunk.len();
            buf.advance(n);
        }
        if let Some(o) = self.observer.as_mut() {
            o.wrote = true;
        }
        Ok(())
    }

    pub async fn read_bytes(&mut self, count: usize) -> FsResult<Bytes> {
        // Pull the next `count` bytes from the resource at the cursor and advance.
        // A short read (or empty at EOF) is honoured verbatim.
        let end = self.offset.saturating_add(count as u64);
        let data = self
            .resource
            .read_bytes_pinned(&self.path, Some(self.offset..end), &self.stat)
            .await
            .map_err(FsError::from)?;
        self.offset = self.offset.saturating_add(data.len() as u64);
        Ok(Bytes::from(data))
    }

    pub async fn seek(&mut self, pos: SeekFrom) -> FsResult<u64> {
        let new = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(d) => (self.offset as i64 + d).max(0) as u64,
            // Size is pinned at open, so seeking from the end needs no stat.
            SeekFrom::End(d) => (self.stat.size as i64 + d).max(0) as u64,
        };
        self.offset = new;
        Ok(new)
    }

    pub async fn flush(&mut self) -> FsResult<()> {
        if !self.write {
            return Ok(());
        }
        // Whole-object write (create/truncate). Clone rather than take so a second
        // flush after further writes rewrites the grown buffer, not an empty one.
        self.resource
            .write_bytes(&self.path, self.wbuf.clone())
            .await
            .map_err(FsError::from)?;
        // Report a completed write once. An empty create (no `write_*`) still
        // creates the file above but fires nothing, matching the prior behaviour.
        if let Some(o) = self.observer.as_mut()
            && o.wrote
        {
            o.wrote = false;
            if let Some(h) = &o.hook {
                let ev = if o.existed {
                    FsEvent::Modified(&o.rel)
                } else {
                    FsEvent::Created(&o.rel)
                };
                h.on_change(ev);
            }
        }
        Ok(())
    }
}

/// A filesystem handle scoped to a single workspace: a router over its own mount
/// table (the local `/files` mount + provider mounts). Cheap to clone
/// (`Arc<[Mount]>` + an optional hook); `Send + Sync + 'static`.
#[derive(Clone)]
pub struct WorkspaceFs {
    /// Mounts sorted by prefix length descending, for longest-prefix routing.
    mounts: Arc<[Mount]>,
    /// Change hook, injected by the host. `None` fires nothing. The host bakes
    /// any identity it needs (e.g. the workspace id) into its `FsHook` impl.
    hook: Option<Arc<dyn FsHook>>,
}

/// Concurrent renders while sizing one listing (see
/// [`WorkspaceFs::read_dir`]). A Drive document takes 1-2s to produce, so a folder
/// of them is unusable serially; past this the provider's rate limits, not latency,
/// are the constraint.
const LISTING_SIZE_CONCURRENCY: usize = 8;

impl WorkspaceFs {
    /// Build from live mounts. Rejects empty/relative or duplicate prefixes and
    /// sorts by prefix length descending so [`Self::route`] does longest-match.
    pub fn from_mounts(mut mounts: Vec<Mount>) -> anyhow::Result<Self> {
        for m in &mounts {
            if m.prefix.is_empty() || !m.prefix.starts_with('/') {
                anyhow::bail!(
                    "mount prefix must be absolute and non-empty: {:?}",
                    m.prefix
                );
            }
        }
        for i in 0..mounts.len() {
            for j in (i + 1)..mounts.len() {
                if mounts[i].prefix == mounts[j].prefix {
                    anyhow::bail!("duplicate mount prefix: {}", mounts[i].prefix);
                }
            }
        }
        mounts.sort_by_key(|mount| std::cmp::Reverse(mount.prefix.len()));
        Ok(Self {
            mounts: mounts.into(),
            hook: None,
        })
    }

    /// Instantiate the resources from `config` (the local `/files` mount when
    /// `local_root` is set, plus provider mounts) and build the filesystem.
    pub fn from_config(config: FsConfig) -> anyhow::Result<Self> {
        Self::from_mounts(build_mounts(config)?)
    }

    /// A local-only workspace filesystem rooted at `root` (no provider mounts) —
    /// a convenience for standalone use and tests.
    pub fn local(root: PathBuf) -> Self {
        Self::from_config(FsConfig {
            local_root: Some(root),
            mirror_root: None,
            mounts: Vec::new(),
        })
        .expect("local-only fs is always valid")
    }

    /// Attach an [`FsHook`] fired on every mutation. `None` clears it.
    pub fn with_hook(mut self, hook: Option<Arc<dyn FsHook>>) -> Self {
        self.hook = hook;
        self
    }

    fn fire(&self, event: FsEvent<'_>) {
        if let Some(h) = &self.hook {
            h.on_change(event);
        }
    }

    /// Whether the mount owning `rel_path` can report a
    /// [`content_type`](crate::vfs::DirEntry::content_type) at all. A prefix
    /// lookup — no provider call — so a frontend can skip asking on the mounts
    /// that would only ever answer "nothing".
    pub fn reports_content_type(&self, rel_path: &str) -> bool {
        self.route(rel_path)
            .map(|(r, _)| r.reports_content_type())
            .unwrap_or(false)
    }

    /// Route a workspace-relative path to the mount that owns it, by longest
    /// prefix. `None` means the path names no node (the virtual root, or a top
    /// segment that is neither the local mount nor a provider prefix).
    fn route(&self, rel_path: &str) -> Option<(Arc<dyn Resource>, MountPath)> {
        let abs = if rel_path.starts_with('/') {
            rel_path.to_string()
        } else {
            format!("/{rel_path}")
        };
        let abs = abs.trim_end_matches('/');
        for m in self.mounts.iter() {
            if abs == m.prefix {
                return Some((Arc::clone(&m.resource), MountPath::root()));
            }
            if let Some(rest) = abs.strip_prefix(&m.prefix)
                && rest.starts_with('/')
            {
                return Some((Arc::clone(&m.resource), MountPath::new(rest)));
            }
        }
        None
    }

    /// Top-level mount names (no leading `/`) — the children of the virtual root.
    pub fn mount_names(&self) -> Vec<String> {
        self.mounts
            .iter()
            .map(|m| m.prefix.trim_start_matches('/').to_string())
            .collect()
    }

    pub async fn metadata(&self, rel_path: &str) -> FsResult<Stat> {
        if is_root_path(rel_path) {
            return Ok(dir_stat());
        }
        let (resource, vpath) = self.route(rel_path).ok_or(FsError::NotFound)?;
        resource
            .stat(&vpath)
            .await
            .map(stat_from_vfs)
            .map_err(FsError::from)
    }

    /// Symlinks aren't modelled, so this is identical to [`Self::metadata`].
    pub async fn symlink_metadata(&self, rel_path: &str) -> FsResult<Stat> {
        self.metadata(rel_path).await
    }

    /// List a directory with every length resolved — for callers that report sizes
    /// (WebDAV `PROPFIND` sends `getcontentlength` for each child).
    ///
    /// A provider that cannot size a file without producing it lists a placeholder
    /// (see [`crate::FileStat::size_is_estimate`]); resolving those means one render
    /// each, so they run concurrently and the listing costs the slowest rather than
    /// their sum. Use [`Self::read_dir_unsized`] where the sizes are discarded.
    pub async fn read_dir(&self, rel_path: &str) -> FsResult<DirStream> {
        self.list(rel_path, true).await
    }

    /// List a directory without resolving placeholder lengths — for callers that
    /// only need names and types (the FUSE forwarder's `readdir`, which parses
    /// exactly those out of the response, and `find`/`ls` above it).
    pub async fn read_dir_unsized(&self, rel_path: &str) -> FsResult<DirStream> {
        self.list(rel_path, false).await
    }

    async fn list(&self, rel_path: &str, with_sizes: bool) -> FsResult<DirStream> {
        // The virtual root lists the mount names as subdirectories.
        if is_root_path(rel_path) {
            let out: Vec<FsResult<DirEntry>> = self
                .mount_names()
                .into_iter()
                .map(|name| Ok(mount_dir_entry(&name)))
                .collect();
            return Ok(Box::pin(stream::iter(out)));
        }
        let (resource, vpath) = self.route(rel_path).ok_or(FsError::NotFound)?;
        let mut entries = resource.readdir(&vpath).await.map_err(FsError::from)?;
        // Only worth a pass if the provider will actually answer: one that declines
        // (Drive, whose answer is a document render) returns the same placeholder,
        // and asking anyway is a stat per entry for a number that cannot change.
        if with_sizes && resource.resolve_size_on_stat() {
            let at: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.size_is_estimate)
                .map(|(i, _)| i)
                .collect();
            let paths: Vec<MountPath> = at.iter().map(|&i| vpath.child(&entries[i].name)).collect();
            // `stat` is what resolves a placeholder (the provider renders once and
            // the cache keeps both bytes and length), so driving it per child is
            // also what warms the reads that follow.
            let sized = futures::StreamExt::collect::<Vec<_>>(futures::StreamExt::buffered(
                stream::iter(paths.into_iter().map(|p| {
                    let resource = resource.clone();
                    async move { resource.stat(&p).await.ok() }
                })),
                LISTING_SIZE_CONCURRENCY,
            ))
            .await;
            for (i, st) in at.into_iter().zip(sized) {
                if let Some(st) = st.filter(|st| !st.size_is_estimate) {
                    entries[i].size = st.size;
                    entries[i].size_is_estimate = false;
                }
            }
        }
        let out: Vec<FsResult<DirEntry>> = entries
            .into_iter()
            .map(|e| Ok(dir_entry_from_vfs(e)))
            .collect();
        Ok(Box::pin(stream::iter(out)))
    }

    pub async fn open(&self, rel_path: &str, options: OpenOptions) -> FsResult<File> {
        let (resource, vpath) = self.route(rel_path).ok_or(FsError::NotFound)?;
        let is_write = options.write || options.append || options.create || options.create_new;

        // One stat serves both the existence probe (create/modify distinction,
        // create_new guard) and the read-open directory/existence check.
        let stat = resource.stat(&vpath).await;
        let existed = stat.is_ok();
        // Pin the open-time snapshot so every chunk of a read validates against
        // one version (no per-chunk stat); default for a not-yet-created file.
        let pinned = stat.as_ref().ok().cloned().unwrap_or_default();
        if options.create_new && existed {
            return Err(FsError::Exists);
        }
        if !is_write {
            match stat {
                Ok(st) if matches!(st.kind, FileKind::Dir) => return Err(FsError::Forbidden),
                Ok(_) => {}
                Err(e) => return Err(FsError::from(e)),
            }
        }

        // Append preserves existing content by seeding the buffer; the flush then
        // rewrites the whole object.
        let mut wbuf = Vec::new();
        if options.append && existed {
            wbuf = resource
                .read_bytes(&vpath, None)
                .await
                .map_err(FsError::from)?;
        }

        let observer = (is_write && self.hook.is_some()).then(|| Observer {
            rel: rel_path.to_string(),
            existed,
            wrote: false,
            hook: self.hook.clone(),
        });
        Ok(File {
            resource,
            path: vpath,
            offset: 0,
            stat: pinned,
            write: is_write,
            wbuf,
            observer,
        })
    }

    pub async fn create_dir(&self, rel_path: &str) -> FsResult<()> {
        let (resource, vpath) = self.route(rel_path).ok_or(FsError::Forbidden)?;
        // A directory create fires no change event (dirs aren't ingested).
        resource.mkdir(&vpath).await.map_err(FsError::from)
    }

    pub async fn remove_dir(&self, rel_path: &str) -> FsResult<()> {
        let (resource, vpath) = self.route(rel_path).ok_or(FsError::NotFound)?;
        resource.rmdir(&vpath).await.map_err(FsError::from)?;
        self.fire(FsEvent::Removed(rel_path));
        Ok(())
    }

    pub async fn remove_file(&self, rel_path: &str) -> FsResult<()> {
        let (resource, vpath) = self.route(rel_path).ok_or(FsError::NotFound)?;
        resource.unlink(&vpath).await.map_err(FsError::from)?;
        self.fire(FsEvent::Removed(rel_path));
        Ok(())
    }

    pub async fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        let (rf, vfrom) = self.route(from).ok_or(FsError::Forbidden)?;
        let (rt, vto) = self.route(to).ok_or(FsError::Forbidden)?;
        // Cross-mount moves aren't supported (can't move between local/provider,
        // or between two providers).
        if !Arc::ptr_eq(&rf, &rt) {
            return Err(FsError::Forbidden);
        }
        let to_existed = rt.stat(&vto).await.is_ok();
        rf.rename(&vfrom, &vto).await.map_err(FsError::from)?;
        self.fire(FsEvent::Removed(from));
        self.fire(if to_existed {
            FsEvent::Modified(to)
        } else {
            FsEvent::Created(to)
        });
        Ok(())
    }

    pub async fn copy(&self, from: &str, to: &str) -> FsResult<()> {
        let (rf, vfrom) = self.route(from).ok_or(FsError::Forbidden)?;
        let (rt, vto) = self.route(to).ok_or(FsError::Forbidden)?;
        if !Arc::ptr_eq(&rf, &rt) {
            return Err(FsError::Forbidden);
        }
        let to_existed = rt.stat(&vto).await.is_ok();
        // No `Resource::copy`; read the whole object and write it back within the
        // same mount.
        let data = rf.read_bytes(&vfrom, None).await.map_err(FsError::from)?;
        rt.write_bytes(&vto, data).await.map_err(FsError::from)?;
        self.fire(if to_existed {
            FsEvent::Modified(to)
        } else {
            FsEvent::Created(to)
        });
        Ok(())
    }
}

/// Serves the unified workspace tree to the in-guest FUSE forwarder. The guest
/// paths (`/files/…` local, `/notion/…` provider, `/` root) are the workspace's
/// own namespace, so every method is `route → delegate`; the root lists the
/// mount names. Writable per the resource's capability (S3 writes objects; a
/// provider whose `write_bytes` is `Unsupported` surfaces the error to the guest).
#[async_trait]
impl ForwardFs for WorkspaceFs {
    async fn readdir(&self, path: &str) -> anyhow::Result<Vec<FwdEntry>> {
        if is_root_path(path) {
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
        // Sizes are dropped by the forwarder (it reads `name` and `is_dir`), so
        // don't pay to resolve them here.
        let mut stream = self
            .read_dir_unsized(path)
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
        Ok(match self.metadata(path).await {
            Ok(st) => FwdStat {
                exists: true,
                is_dir: st.is_dir(),
                size: st.len,
                mtime: st.modified.and_then(secs_since_epoch),
                atime: st.accessed.and_then(secs_since_epoch),
                ctime: st.status_changed.and_then(secs_since_epoch),
            },
            // Any stat failure (missing path, provider error) reads as ENOENT.
            Err(_) => FwdStat::missing(),
        })
    }

    async fn read(
        &self,
        path: &str,
        offset: Option<u64>,
        size: Option<u64>,
    ) -> anyhow::Result<Vec<u8>> {
        let mut file = self
            .open(
                path,
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
        // `read_bytes` may short-read (a mount pulls one range at a time), so loop
        // until the requested size is met or EOF. A `None` size reads to EOF in
        // bounded chunks.
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
        // Whole-file replace (create+truncate), matching the guest forwarder's
        // flush semantics. The resource decides writability.
        let mut f = self
            .open(
                path,
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
        Ok(b"{\"ok\":true}".to_vec())
    }

    async fn unlink(&self, path: &str) -> anyhow::Result<()> {
        self.remove_file(path)
            .await
            .map_err(|e| anyhow::anyhow!("unlink {path}: {e:?}"))
    }

    async fn mkdir(&self, path: &str) -> anyhow::Result<()> {
        self.create_dir(path)
            .await
            .map_err(|e| anyhow::anyhow!("mkdir {path}: {e:?}"))
    }

    async fn rmdir(&self, path: &str) -> anyhow::Result<()> {
        self.remove_dir(path)
            .await
            .map_err(|e| anyhow::anyhow!("rmdir {path}: {e:?}"))
    }

    async fn rename(&self, from: &str, to: &str) -> anyhow::Result<()> {
        // The inherent `WorkspaceFs::rename` wins method resolution over this
        // trait method, so this is not recursive.
        WorkspaceFs::rename(self, from, to)
            .await
            .map_err(|e| anyhow::anyhow!("rename {from} -> {to}: {e:?}"))
    }
}

/// True for the workspace root (`/` or empty), the virtual container of mounts.
fn is_root_path(rel_path: &str) -> bool {
    rel_path.trim_matches('/').is_empty()
}

/// Convert a provider directory entry into a workspace [`DirEntry`], carrying the
/// kind/size/timestamps the provider reported.
fn dir_entry_from_vfs(e: crate::vfs::DirEntry) -> DirEntry {
    let stat = Stat {
        kind: node_kind_of(e.kind),
        len: e.size,
        modified: e.mtime,
        accessed: e.atime,
        created: e.created,
        status_changed: e.ctime,
        executable: None,
        content_type: e.content_type,
    };
    DirEntry {
        name: e.name.into_bytes(),
        metadata: Ok(stat),
    }
}

/// A synthetic directory entry for a mount name, listed at the workspace root.
fn mount_dir_entry(name: &str) -> DirEntry {
    DirEntry {
        name: name.as_bytes().to_vec(),
        metadata: Ok(dir_stat()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::SeekFrom;
    use std::ops::Range;
    use std::sync::Arc;

    use async_trait::async_trait;
    use bytes::Bytes;
    use futures_util::StreamExt;

    use super::{FsError, OpenOptions, WorkspaceFs};
    use crate::vfs::{
        DirEntry as ResourceDirEntry, FileKind, FileStat, LocalResource, Mount, MountPath,
        Resource, ResourceError, ResourceResult,
    };

    /// A tiny read-only provider holding one file, `/file.txt`.
    struct MockResource {
        content: Vec<u8>,
    }

    #[async_trait]
    impl Resource for MockResource {
        async fn read_bytes(
            &self,
            path: &MountPath,
            range: Option<Range<u64>>,
        ) -> ResourceResult<Vec<u8>> {
            if path.as_str() != "/file.txt" {
                return Err(ResourceError::NotFound);
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

        async fn write_bytes(&self, _path: &MountPath, _data: Vec<u8>) -> ResourceResult<()> {
            Err(ResourceError::Unsupported)
        }

        async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<ResourceDirEntry>> {
            if path.is_root() {
                Ok(vec![ResourceDirEntry {
                    name: "file.txt".to_string(),
                    kind: FileKind::File,
                    size: self.content.len() as u64,
                    mtime: None,
                    atime: None,
                    ctime: None,
                    created: None,
                    etag: None,
                    content_type: None,
                    size_is_estimate: false,
                    serves_whole: false,
                }])
            } else {
                Err(ResourceError::NotFound)
            }
        }

        async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
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
                Err(ResourceError::NotFound)
            }
        }
    }

    /// A `WorkspaceFs` with the local tree at `/files` plus a mock provider at
    /// `/mock`.
    fn mounted_fs(root: std::path::PathBuf) -> WorkspaceFs {
        let local: Arc<dyn Resource> = Arc::new(LocalResource::new(root));
        let mock: Arc<dyn Resource> = Arc::new(MockResource {
            content: b"hello world".to_vec(),
        });
        WorkspaceFs::from_mounts(vec![
            Mount {
                prefix: "/files".to_string(),
                resource: local,
            },
            Mount {
                prefix: "/mock".to_string(),
                resource: mock,
            },
        ])
        .unwrap()
    }

    async fn dir_names(fs: &WorkspaceFs, path: &str) -> Vec<String> {
        let mut stream = fs.read_dir(path).await.unwrap();
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
        let head = f.read_bytes(11).await.unwrap();
        assert_eq!(&head[..], b"hello world");

        f.seek(SeekFrom::Start(6)).await.unwrap();
        let tail = f.read_bytes(5).await.unwrap();
        assert_eq!(&tail[..], b"world");
    }

    #[tokio::test]
    async fn root_lists_mount_names() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("local.txt"), b"x")
            .await
            .unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        // The root lists the mount names — local files are under `/files`, not root.
        assert_eq!(dir_names(&fs, "/").await, vec!["files", "mock"]);
        // The local mount lists local entries; the mock lists provider entries.
        assert_eq!(dir_names(&fs, "/files").await, vec!["local.txt"]);
        assert_eq!(dir_names(&fs, "/mock").await, vec!["file.txt"]);
    }

    #[tokio::test]
    async fn local_paths_live_under_files() {
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("local.txt"), b"data")
            .await
            .unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        let st = fs.metadata("/files/local.txt").await.unwrap();
        assert!(st.is_file());
        assert_eq!(st.len, 4);
        // A bare root-relative path (no mount prefix) names no node.
        assert_eq!(
            fs.metadata("/local.txt").await.err(),
            Some(FsError::NotFound)
        );
    }

    #[tokio::test]
    async fn provider_mount_mutations_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        // Write-open on a provider now succeeds (deferred failure); the write
        // itself surfaces the provider's `Unsupported` at flush.
        let mut f = fs
            .open(
                "/mock/file.txt",
                OpenOptions {
                    write: true,
                    create: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        f.write_bytes(Bytes::from_static(b"x")).await.unwrap();
        assert_eq!(f.flush().await.err(), Some(FsError::NotImplemented));

        // Directory / delete / rename ops the mock doesn't implement → NotImplemented.
        assert_eq!(
            fs.create_dir("/mock/x").await.err(),
            Some(FsError::NotImplemented)
        );
        assert_eq!(
            fs.remove_file("/mock/file.txt").await.err(),
            Some(FsError::NotImplemented)
        );
        assert_eq!(
            fs.rename("/mock/a", "/mock/b").await.err(),
            Some(FsError::NotImplemented)
        );
        // Crossing the local/provider boundary is forbidden.
        assert_eq!(
            fs.copy("/files/x", "/mock/b").await.err(),
            Some(FsError::Forbidden)
        );
    }

    /// The `ForwardFs` view serves the same unified tree: the mount names at the
    /// root, local files under `/files`, provider objects under `/mock`.
    #[tokio::test]
    async fn forward_fs_serves_unified_tree() {
        use crate::vfs::sandbox::ForwardFs;

        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(tmp.path().join("local.txt"), b"local-bytes")
            .await
            .unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        // Root lists the mount names.
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

        // Local files live under `/files`; the mount lists provider entries.
        let local: Vec<String> = ForwardFs::readdir(&fs, "/files")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(local, vec!["local.txt".to_string()]);
        let mount: Vec<(String, bool)> = ForwardFs::readdir(&fs, "/mock")
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        assert_eq!(mount, vec![("file.txt".to_string(), false)]);

        // Stat crosses both worlds.
        assert!(ForwardFs::stat(&fs, "/files").await.unwrap().is_dir);
        let ls = ForwardFs::stat(&fs, "/files/local.txt").await.unwrap();
        assert!(ls.exists && !ls.is_dir && ls.size == 11);
        // Local files expose real mtime/atime/ctime from disk metadata.
        assert!(ls.mtime.is_some() && ls.atime.is_some() && ls.ctime.is_some());
        let ms = ForwardFs::stat(&fs, "/mock/file.txt").await.unwrap();
        assert!(ms.exists && !ms.is_dir && ms.size == 11);
        assert!(!ForwardFs::stat(&fs, "/nope").await.unwrap().exists);
        // A bare root-relative path is not a node.
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

        // Local writes succeed; the mock provider stays read-only.
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

    /// The local `/files` mount is read-write through the unified `ForwardFs`
    /// view — write / mkdir / rename / unlink land on disk; provider paths stay
    /// read-only and crossing the boundary is rejected.
    #[tokio::test]
    async fn forward_fs_local_write() {
        use crate::vfs::sandbox::ForwardFs;

        let tmp = tempfile::tempdir().unwrap();
        let fs = mounted_fs(tmp.path().to_path_buf());

        ForwardFs::write(&fs, "/files/note.txt", b"hello".to_vec())
            .await
            .unwrap();
        assert_eq!(
            ForwardFs::read(&fs, "/files/note.txt", None, None)
                .await
                .unwrap(),
            b"hello"
        );
        assert!(tmp.path().join("note.txt").exists());

        // Whole-file overwrite.
        ForwardFs::write(&fs, "/files/note.txt", b"world!".to_vec())
            .await
            .unwrap();
        assert_eq!(
            ForwardFs::read(&fs, "/files/note.txt", None, None)
                .await
                .unwrap(),
            b"world!"
        );

        // mkdir + nested write, then it lists.
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

        // rename + unlink within the local mount.
        ForwardFs::rename(&fs, "/files/note.txt", "/files/renamed.txt")
            .await
            .unwrap();
        assert!(
            ForwardFs::stat(&fs, "/files/renamed.txt")
                .await
                .unwrap()
                .exists
        );
        assert!(
            !ForwardFs::stat(&fs, "/files/note.txt")
                .await
                .unwrap()
                .exists
        );
        ForwardFs::unlink(&fs, "/files/renamed.txt").await.unwrap();
        assert!(
            !ForwardFs::stat(&fs, "/files/renamed.txt")
                .await
                .unwrap()
                .exists
        );

        // Provider stays read-only, and crossing the boundary is rejected.
        assert!(
            ForwardFs::write(&fs, "/mock/file.txt", b"x".to_vec())
                .await
                .is_err()
        );
        assert!(ForwardFs::mkdir(&fs, "/mock/newdir").await.is_err());
        assert!(ForwardFs::rename(&fs, "/files/a", "/mock/b").await.is_err());
    }
    /// The cost split: a listing that reports lengths renders what it cannot size,
    /// and one that does not report them renders nothing. `find` and the FUSE
    /// forwarder's `readdir` take the second path — measured against a real Drive,
    /// resolving six documents cost 2.4s that neither of them reads.
    #[tokio::test]
    async fn only_a_listing_that_reports_lengths_pays_to_learn_them() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Folder {
            reads: Arc<AtomicUsize>,
        }
        #[async_trait]
        impl Resource for Folder {
            async fn read_bytes(
                &self,
                _p: &MountPath,
                _r: Option<Range<u64>>,
            ) -> ResourceResult<Vec<u8>> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                Ok(b"rendered".to_vec())
            }
            async fn write_bytes(&self, _p: &MountPath, _d: Vec<u8>) -> ResourceResult<()> {
                Err(ResourceError::Unsupported)
            }
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<ResourceDirEntry>> {
                Ok(vec![ResourceDirEntry {
                    name: "doc.gdoc.json".to_string(),
                    kind: FileKind::File,
                    size: 64 << 20,
                    size_is_estimate: true,
                    serves_whole: false,
                    mtime: Some(std::time::UNIX_EPOCH),
                    atime: None,
                    ctime: None,
                    created: None,
                    etag: None,
                    content_type: None,
                }])
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
                Ok(FileStat {
                    kind: FileKind::File,
                    size: 64 << 20,
                    size_is_estimate: true,
                    serves_whole: false,
                    mtime: Some(std::time::UNIX_EPOCH),
                    ..Default::default()
                })
            }
        }

        async fn listed(fs: &WorkspaceFs, sized: bool) -> crate::Stat {
            let mut s = if sized {
                fs.read_dir("/m").await.unwrap()
            } else {
                fs.read_dir_unsized("/m").await.unwrap()
            };
            s.next().await.unwrap().unwrap().metadata().unwrap()
        }

        for sized in [false, true] {
            let reads = Arc::new(AtomicUsize::new(0));
            let fs = WorkspaceFs::from_mounts(vec![Mount {
                prefix: "/m".to_string(),
                resource: Arc::new(crate::vfs::cache::CachedResource::new(Arc::new(Folder {
                    reads: reads.clone(),
                }))),
            }])
            .unwrap();

            let st = listed(&fs, sized).await;
            if sized {
                assert_eq!(st.len, 8, "the rendered length");
                assert_eq!(reads.load(Ordering::SeqCst), 1, "rendered once");
            } else {
                assert_eq!(st.len, 64 << 20, "the placeholder stands");
                assert_eq!(reads.load(Ordering::SeqCst), 0, "nothing rendered");
            }
        }
    }

    /// An estimated size is fine for a listing and wrong for an open handle: a
    /// WebDAV `GET` fills `Content-Length` from the handle, so it must be exact,
    /// while `ls -l` must not render a folder's worth of documents to find out.
    /// The handle therefore resolves it — once, through whatever cache sits below
    /// — and a provider that reports real sizes never pays for the check.
    #[tokio::test]
    async fn an_open_handle_resolves_an_estimated_size() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Estimating {
            reads: Arc<AtomicUsize>,
            estimate: bool,
        }
        #[async_trait]
        impl Resource for Estimating {
            async fn read_bytes(
                &self,
                _p: &MountPath,
                _r: Option<Range<u64>>,
            ) -> ResourceResult<Vec<u8>> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                Ok(b"converted".to_vec())
            }
            async fn write_bytes(&self, _p: &MountPath, _d: Vec<u8>) -> ResourceResult<()> {
                Err(ResourceError::Unsupported)
            }
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<ResourceDirEntry>> {
                Ok(Vec::new())
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
                Ok(FileStat {
                    kind: FileKind::File,
                    // An upper bound, far above the 9 bytes a read returns.
                    size: if self.estimate { 64 << 20 } else { 9 },
                    size_is_estimate: self.estimate,
                    serves_whole: false,
                    ..Default::default()
                })
            }
        }

        for (estimate, want_reads) in [(true, 1), (false, 0)] {
            let reads = Arc::new(AtomicUsize::new(0));
            let fs = WorkspaceFs::from_mounts(vec![Mount {
                prefix: "/m".to_string(),
                resource: Arc::new(Estimating {
                    reads: reads.clone(),
                    estimate,
                }),
            }])
            .unwrap();

            // A plain `stat` keeps the estimate and reads nothing.
            let st = fs.metadata("/m/doc.txt").await.unwrap();
            assert_eq!(st.len, if estimate { 64 << 20 } else { 9 });
            assert_eq!(reads.load(Ordering::SeqCst), 0, "stat must not fetch");

            // Opening it and asking makes the length exact.
            let mut f = fs
                .open(
                    "/m/doc.txt",
                    OpenOptions {
                        read: true,
                        ..Default::default()
                    },
                )
                .await
                .expect("open");
            assert_eq!(f.metadata().await.unwrap().len, 9, "estimate={estimate}");
            assert_eq!(
                reads.load(Ordering::SeqCst),
                want_reads,
                "estimate={estimate}: resolving should{} fetch",
                if estimate { "" } else { " not" }
            );
        }
    }

    /// The capability gate: whether asking a mount for a subject type is worth a
    /// call at all. It has to answer from the prefix alone — the WebDAV layer
    /// consults it per node while building a PROPFIND, so a provider that never
    /// has an answer must not cost a `metadata()` to say so.
    #[test]
    fn reports_content_type_is_answered_by_the_mount_not_a_stat() {
        struct Typed;
        #[async_trait]
        impl Resource for Typed {
            async fn read_bytes(
                &self,
                _p: &MountPath,
                _r: Option<Range<u64>>,
            ) -> ResourceResult<Vec<u8>> {
                Err(ResourceError::NotFound)
            }
            async fn write_bytes(&self, _p: &MountPath, _d: Vec<u8>) -> ResourceResult<()> {
                Err(ResourceError::Unsupported)
            }
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<ResourceDirEntry>> {
                Ok(Vec::new())
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
                Err(ResourceError::NotFound)
            }
            fn reports_content_type(&self) -> bool {
                true
            }
        }

        let fs = WorkspaceFs::from_mounts(vec![
            Mount {
                prefix: "/typed".to_string(),
                resource: Arc::new(Typed),
            },
            Mount {
                prefix: "/plain".to_string(),
                resource: Arc::new(MockResource {
                    content: b"x".to_vec(),
                }),
            },
        ])
        .unwrap();

        assert!(fs.reports_content_type("/typed"));
        assert!(fs.reports_content_type("/typed/deep/file.pdf.json"));
        // The default: a provider whose filenames already carry the type.
        assert!(!fs.reports_content_type("/plain/file.txt"));
        // And nothing at all outside the mounts (the virtual root).
        assert!(!fs.reports_content_type("/"));
        assert!(!fs.reports_content_type("/nope/x"));
    }
}
