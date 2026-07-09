//! In-memory metadata index. A `readdir` populates it with each child's
//! type + size; a `stat` reads from it (fast-path hit + negative caching) so
//! the per-entry `getattr` storm the kernel issues after a listing (e.g.
//! `ls -la`) costs no provider round trips. [`CachedResource`] wraps a provider
//! [`Resource`] so both FUSE frontends benefit transparently.

use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;

use crate::vfs::{
    error::{VfsError, VfsResult},
    path::VPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

/// Directory-listing TTL. Listings have no cheap version to revalidate against
/// (unlike file content), so they fall back to a short expiry.
const DEFAULT_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Copy)]
struct Entry {
    is_dir: bool,
    size: u64,
    /// Per-entry times carried from `readdir` so the stat fast-path (`ls -l`)
    /// returns real times instead of the epoch (R2). `atime`/`ctime` are `None`
    /// for backends that don't report them (e.g. S3).
    mtime: Option<std::time::SystemTime>,
    atime: Option<std::time::SystemTime>,
    ctime: Option<std::time::SystemTime>,
}

#[derive(Default)]
struct Inner {
    /// path -> metadata.
    entries: HashMap<String, Entry>,
    /// directory path -> its child full-paths.
    children: HashMap<String, Vec<String>>,
    /// directory path -> listing expiry.
    expiry: HashMap<String, Instant>,
}

/// Per-mount metadata index. Only directory listings expire (TTL); individual
/// entries live until their listing is overwritten or invalidated.
struct IndexCache {
    inner: Mutex<Inner>,
    ttl: Duration,
}

impl IndexCache {
    fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            ttl,
        }
    }

    /// Entry metadata for a path, if known.
    fn get(&self, path: &str) -> Option<Entry> {
        self.inner.lock().unwrap().entries.get(path).copied()
    }

    /// Whether `path`'s listing is present and unexpired (basis for negative
    /// caching: a fresh listing means non-members provably don't exist).
    fn is_listed(&self, path: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        matches!(inner.expiry.get(path), Some(exp) if *exp > Instant::now())
    }

    /// Reconstruct a directory's entries from the cache if its listing is
    /// fresh; `None` means not listed or expired (caller must hit the network).
    /// Done under one lock so the listing and its entries stay consistent.
    fn list_dir_entries(&self, path: &str) -> Option<Vec<DirEntry>> {
        let inner = self.inner.lock().unwrap();
        match inner.expiry.get(path) {
            Some(exp) if *exp > Instant::now() => {}
            _ => return None,
        }
        let children = inner.children.get(path)?;
        let out = children
            .iter()
            .filter_map(|full| {
                inner.entries.get(full).map(|e| DirEntry {
                    name: basename(full).to_string(),
                    kind: if e.is_dir {
                        FileKind::Dir
                    } else {
                        FileKind::File
                    },
                    size: e.size,
                    mtime: e.mtime,
                    atime: e.atime,
                    ctime: e.ctime,
                })
            })
            .collect();
        Some(out)
    }

    /// Record a directory listing: store each child's metadata + the child set
    /// with a fresh TTL.
    fn set_dir(&self, path: &str, entries: &[DirEntry]) {
        let prefix = if path == "/" {
            "/".to_string()
        } else {
            format!("{path}/")
        };
        let mut inner = self.inner.lock().unwrap();
        let mut child_keys = Vec::with_capacity(entries.len());
        for e in entries {
            let full = format!("{prefix}{}", e.name);
            inner.entries.insert(
                full.clone(),
                Entry {
                    is_dir: matches!(e.kind, FileKind::Dir),
                    size: e.size,
                    mtime: e.mtime,
                    atime: e.atime,
                    ctime: e.ctime,
                },
            );
            child_keys.push(full);
        }
        inner.children.insert(path.to_string(), child_keys);
        inner
            .expiry
            .insert(path.to_string(), Instant::now() + self.ttl);
    }

    fn invalidate_dir(&self, path: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(children) = inner.children.remove(path) {
            for c in children {
                inner.entries.remove(&c);
            }
        }
        inner.expiry.remove(path);
    }

    /// Invalidate the listing of `path`'s parent directory (so a created /
    /// removed / resized child is re-listed).
    fn invalidate_parent(&self, path: &str) {
        self.invalidate_dir(parent_of(path));
    }

    fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.entries.clear();
        inner.children.clear();
        inner.expiry.clear();
    }
}

/// Full object bytes cached above this size are dropped — a ranged read of a
/// known-large object is served ranged and uncached instead (8 MiB).
const MAX_CONTENT_BYTES: u64 = 8 << 20;

/// A cheap validator for a file's content: the backend's strong tag
/// (S3 `ETag`/`VersionId`) if any, else its mtime (Notion `last_edited_time`).
/// `Unknown` = nothing to validate against, so content is never cache-served.
#[derive(Clone, PartialEq)]
enum Version {
    Tag(String),
    Time(std::time::SystemTime),
    Unknown,
}

impl Version {
    fn of(s: &FileStat) -> Self {
        if let Some(e) = &s.etag {
            Version::Tag(e.clone())
        } else if let Some(v) = &s.version {
            Version::Tag(v.clone())
        } else if let Some(t) = s.mtime {
            Version::Time(t)
        } else {
            Version::Unknown
        }
    }
    fn known(&self) -> bool {
        !matches!(self, Version::Unknown)
    }
}

/// A cached object: (validator, full bytes).
type ContentEntry = (Version, Arc<Vec<u8>>);

/// Wraps a provider [`Resource`] with an [`IndexCache`] for listings (short TTL)
/// plus a **validated** content cache: a read probes the object's cheap version
/// via `stat` and serves cached bytes only while it matches, so external edits
/// are refetched without a TTL wait. Mutations invalidate both.
pub struct CachedResource {
    inner: Arc<dyn Resource>,
    cache: IndexCache,
    /// path -> (validator, full bytes). Held until the validator changes or the
    /// path is written; freshness comes from revalidation, not a TTL.
    content: Mutex<HashMap<String, ContentEntry>>,
}

impl CachedResource {
    pub fn new(inner: Arc<dyn Resource>) -> Self {
        Self {
            inner,
            cache: IndexCache::new(DEFAULT_TTL),
            content: Mutex::new(HashMap::new()),
        }
    }

    /// Cached bytes for `key` iff the stored validator equals `version` (and is
    /// known) — the metadata dirty check; a changed backend version misses.
    fn content_get(&self, key: &str, version: &Version) -> Option<Arc<Vec<u8>>> {
        if !version.known() {
            return None;
        }
        match self.content.lock().unwrap().get(key) {
            Some((v, b)) if v == version => Some(b.clone()),
            _ => None,
        }
    }

    fn content_put(&self, key: &str, version: Version, bytes: Vec<u8>) {
        if version.known() && bytes.len() as u64 <= MAX_CONTENT_BYTES {
            self.content
                .lock()
                .unwrap()
                .insert(key.to_string(), (version, Arc::new(bytes)));
        }
    }

    fn content_drop(&self, key: &str) {
        self.content.lock().unwrap().remove(key);
    }

    fn content_clear(&self) {
        self.content.lock().unwrap().clear();
    }
}

/// Slice `data` by an optional byte range (clamped to bounds); `None` = all.
fn slice(data: &[u8], range: &Option<Range<u64>>) -> Vec<u8> {
    match range {
        Some(r) => {
            let start = (r.start as usize).min(data.len());
            let end = (r.end as usize).min(data.len()).max(start);
            data[start..end].to_vec()
        }
        None => data.to_vec(),
    }
}

#[async_trait]
impl Resource for CachedResource {
    async fn read_bytes(&self, path: &VPath, range: Option<Range<u64>>) -> VfsResult<Vec<u8>> {
        let key = path.as_str();
        // Probe the cheap version; serve cached bytes only while it matches.
        let st = self.inner.stat(path).await?;
        let version = Version::of(&st);
        if let Some(full) = self.content_get(key, &version) {
            return Ok(slice(&full, &range));
        }
        // Miss: fetch + cache the whole object, unless it's a known-large object
        // read with a range (served ranged, uncached). Providers that render the
        // whole file regardless of range report size 0, so they still cache whole.
        let fetch_full = range.is_none() || st.size == 0 || st.size <= MAX_CONTENT_BYTES;
        if fetch_full {
            let full = self.inner.read_bytes(path, None).await?;
            let out = slice(&full, &range);
            self.content_put(key, version, full);
            Ok(out)
        } else {
            self.inner.read_bytes(path, range).await
        }
    }

    async fn write_bytes(&self, path: &VPath, data: Vec<u8>) -> VfsResult<()> {
        let r = self.inner.write_bytes(path, data).await;
        if r.is_ok() {
            self.cache.invalidate_parent(path.as_str());
            self.content_drop(path.as_str());
        }
        r
    }

    async fn readdir(&self, path: &VPath) -> VfsResult<Vec<DirEntry>> {
        if let Some(entries) = self.cache.list_dir_entries(path.as_str()) {
            return Ok(entries);
        }
        let entries = self.inner.readdir(path).await?;
        self.cache.set_dir(path.as_str(), &entries);
        Ok(entries)
    }

    async fn stat(&self, path: &VPath) -> VfsResult<FileStat> {
        let key = path.as_str();
        match self.cache.get(key) {
            Some(e) if e.is_dir => {
                return Ok(FileStat {
                    kind: FileKind::Dir,
                    mtime: e.mtime,
                    atime: e.atime,
                    ctime: e.ctime,
                    ..Default::default()
                });
            }
            // A file with a known (>0) size: serve it (with the cached times so
            // `ls -l` shows real times, not the epoch — R2).
            Some(e) if e.size > 0 => {
                return Ok(FileStat {
                    kind: FileKind::File,
                    size: e.size,
                    mtime: e.mtime,
                    atime: e.atime,
                    ctime: e.ctime,
                    ..Default::default()
                });
            }
            // A file with size 0 is ambiguous: providers report 0 for sizes
            // they don't cheaply know (rendered Notion page.json, exported
            // Google docs). Don't trust it — compute via the provider so reads
            // aren't clamped to nothing. (A genuinely empty file just re-stats.)
            Some(_) => return self.inner.stat(path).await,
            None => {
                // Negative cache: a fresh parent listing that lacks this path
                // proves it does not exist — skip the network probe.
                if !path.is_root() && self.cache.is_listed(parent_of(key)) {
                    return Err(VfsError::NotFound);
                }
            }
        }
        self.inner.stat(path).await
    }

    async fn unlink(&self, path: &VPath) -> VfsResult<()> {
        let r = self.inner.unlink(path).await;
        if r.is_ok() {
            // The path itself may have been a directory; drop its listing too.
            self.cache.invalidate_dir(path.as_str());
            self.cache.invalidate_parent(path.as_str());
            self.content_drop(path.as_str());
        }
        r
    }

    async fn mkdir(&self, path: &VPath) -> VfsResult<()> {
        let r = self.inner.mkdir(path).await;
        if r.is_ok() {
            self.cache.invalidate_parent(path.as_str());
        }
        r
    }

    async fn rmdir(&self, path: &VPath) -> VfsResult<()> {
        let r = self.inner.rmdir(path).await;
        if r.is_ok() {
            self.cache.invalidate_dir(path.as_str());
            self.cache.invalidate_parent(path.as_str());
        }
        r
    }

    async fn rename(&self, from: &VPath, to: &VPath) -> VfsResult<()> {
        let r = self.inner.rename(from, to).await;
        if r.is_ok() {
            self.cache.invalidate_dir(from.as_str());
            self.cache.invalidate_parent(from.as_str());
            self.cache.invalidate_parent(to.as_str());
            self.content_drop(from.as_str());
            self.content_drop(to.as_str());
        }
        r
    }

    async fn command(&self, name: &str, body: &[u8]) -> VfsResult<Vec<u8>> {
        let r = self.inner.command(name, body).await;
        // A domain write (e.g. Notion page-create) may change listings and
        // content anywhere in the mount; conservatively drop both caches.
        if r.is_ok() {
            self.cache.clear();
            self.content_clear();
        }
        r
    }

    fn prompt(&self) -> &str {
        self.inner.prompt()
    }
}

/// Parent directory of an absolute mount-relative path: `/a/b` -> `/a`,
/// `/a` -> `/`, `/` -> `/`.
fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((parent, _)) => parent,
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn de(name: &str, kind: FileKind, size: u64) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            kind,
            size,
            mtime: None,
            atime: None,
            ctime: None,
        }
    }

    #[test]
    fn set_dir_then_get_and_list() {
        let c = IndexCache::new(Duration::from_secs(600));
        c.set_dir(
            "/",
            &[de("a.txt", FileKind::File, 10), de("sub", FileKind::Dir, 0)],
        );
        // stat fast-path
        let a = c.get("/a.txt").unwrap();
        assert!(!a.is_dir && a.size == 10);
        assert!(c.get("/sub").unwrap().is_dir);
        // listing reconstruction
        let listed = c.list_dir_entries("/").unwrap();
        assert_eq!(listed.len(), 2);
        // negative cache basis
        assert!(c.is_listed("/"));
        assert!(c.get("/missing.txt").is_none());
    }

    #[test]
    fn nested_paths_and_parent() {
        assert_eq!(parent_of("/a/b"), "/a");
        assert_eq!(parent_of("/a"), "/");
        assert_eq!(parent_of("/"), "/");
        let c = IndexCache::new(Duration::from_secs(600));
        c.set_dir("/sub", &[de("c.txt", FileKind::File, 5)]);
        assert_eq!(c.get("/sub/c.txt").unwrap().size, 5);
        assert_eq!(basename("/sub/c.txt"), "c.txt");
    }

    #[test]
    fn invalidation() {
        let c = IndexCache::new(Duration::from_secs(600));
        c.set_dir("/", &[de("a.txt", FileKind::File, 1)]);
        c.invalidate_parent("/a.txt"); // parent of /a.txt is /
        assert!(!c.is_listed("/"));
        assert!(c.get("/a.txt").is_none());
    }

    // R2: the stat fast-path reads mtime from the cached Entry, which is
    // populated from the DirEntry carried by readdir. Verify mtime survives
    // set_dir -> get (fast-path source) and -> list_dir_entries (reconstruction).
    #[test]
    fn mtime_carried_through_cache() {
        use std::time::{Duration as D, UNIX_EPOCH};
        let t = UNIX_EPOCH + D::from_secs(1_700_000_000);
        let c = IndexCache::new(Duration::from_secs(600));
        c.set_dir(
            "/",
            &[DirEntry {
                name: "a.txt".to_string(),
                kind: FileKind::File,
                size: 10,
                mtime: Some(t),
                atime: None,
                ctime: None,
            }],
        );
        // fast-path source: get() returns the stored mtime (not None/epoch).
        assert_eq!(c.get("/a.txt").unwrap().mtime, Some(t));
        // listing reconstruction also carries it.
        let listed = c.list_dir_entries("/").unwrap();
        assert_eq!(listed[0].mtime, Some(t));
    }

    #[test]
    fn expiry() {
        let c = IndexCache::new(Duration::from_millis(0));
        c.set_dir("/", &[de("a.txt", FileKind::File, 1)]);
        // TTL 0 -> already expired
        assert!(!c.is_listed("/"));
        assert!(c.list_dir_entries("/").is_none());
    }

    // Validated content cache: repeat reads (incl. ranged) hit the once-fetched
    // object; a changed backend version forces a refetch; a write invalidates.
    // The counting provider proves `read_bytes` fires only on a genuine miss.
    #[tokio::test]
    async fn content_cache_validates_and_invalidates() {
        use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
        use std::time::{Duration as D, UNIX_EPOCH};

        struct Counter {
            reads: AtomicUsize,
            /// Backend version → `stat` mtime; bumping it simulates an edit made
            /// outside the cache.
            ver: AtomicU64,
            data: Mutex<Vec<u8>>,
        }
        #[async_trait]
        impl Resource for Counter {
            async fn read_bytes(
                &self,
                _p: &VPath,
                range: Option<Range<u64>>,
            ) -> VfsResult<Vec<u8>> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                Ok(slice(self.data.lock().unwrap().as_slice(), &range))
            }
            async fn write_bytes(&self, _p: &VPath, data: Vec<u8>) -> VfsResult<()> {
                *self.data.lock().unwrap() = data;
                Ok(())
            }
            async fn readdir(&self, _p: &VPath) -> VfsResult<Vec<DirEntry>> {
                Ok(vec![])
            }
            async fn stat(&self, _p: &VPath) -> VfsResult<FileStat> {
                // Size unknown (0), like Notion — validated by mtime instead.
                Ok(FileStat {
                    kind: FileKind::File,
                    size: 0,
                    mtime: Some(UNIX_EPOCH + D::from_secs(self.ver.load(Ordering::SeqCst))),
                    ..Default::default()
                })
            }
        }

        let counter = Arc::new(Counter {
            reads: AtomicUsize::new(0),
            ver: AtomicU64::new(1),
            data: Mutex::new(b"hello world".to_vec()),
        });
        let cached = CachedResource::new(counter.clone());
        let p = VPath::new("/f.txt");

        // First read fetches + caches (validated by version 1).
        assert_eq!(cached.read_bytes(&p, None).await.unwrap(), b"hello world");
        // Ranged repeat at the same version → served from cache, inner not hit.
        assert_eq!(cached.read_bytes(&p, Some(0..5)).await.unwrap(), b"hello");
        assert_eq!(
            counter.reads.load(Ordering::SeqCst),
            1,
            "same version → cache hit"
        );

        // External edit: change bytes + bump the version WITHOUT going through
        // the cache. The dirty check must notice and refetch.
        *counter.data.lock().unwrap() = b"edited elsewhere".to_vec();
        counter.ver.store(2, Ordering::SeqCst);
        assert_eq!(
            cached.read_bytes(&p, None).await.unwrap(),
            b"edited elsewhere"
        );
        assert_eq!(
            counter.reads.load(Ordering::SeqCst),
            2,
            "changed version → refetch"
        );

        // A write through the cache invalidates the entry too.
        cached.write_bytes(&p, b"written".to_vec()).await.unwrap();
        assert_eq!(cached.read_bytes(&p, None).await.unwrap(), b"written");
        assert_eq!(counter.reads.load(Ordering::SeqCst), 3, "write invalidates");
    }
}
