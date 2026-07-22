//! In-memory metadata index. A `readdir` populates it with each child's
//! type + size; a `stat` reads from it (fast-path hit + negative caching) so
//! the per-entry `getattr` storm the kernel issues after a listing (e.g.
//! `ls -la`) costs no provider round trips. [`CachedResource`] wraps a provider
//! [`Resource`] so both FUSE frontends benefit transparently.

use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;

use crate::vfs::{
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, Resource, SEARCH_DIR},
};

/// Directory-listing TTL. Listings have no cheap version to revalidate against
/// (unlike file content), so they fall back to a short expiry.
const LISTING_TTL: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct Entry {
    is_dir: bool,
    size: u64,
    /// Per-entry times carried from `readdir` so the stat fast-path (`ls -l`)
    /// returns real times instead of the epoch (R2). `atime`/`ctime` are `None`
    /// for backends that don't report them (e.g. S3).
    mtime: Option<std::time::SystemTime>,
    atime: Option<std::time::SystemTime>,
    ctime: Option<std::time::SystemTime>,
    /// Strong version tag (S3 `ETag`) carried from the listing, so the stat
    /// fast-path can hand `open` a pin-able snapshot (`If-Match`).
    etag: Option<String>,
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

    /// Entry metadata for a path, if its owning listing (the parent directory)
    /// is still fresh. An entry outlives its listing only until the next access:
    /// a stale/expired one is dropped here and reported as absent, so a deleted
    /// or TTL-expired object stops stat'ing as present.
    fn get(&self, path: &str) -> Option<Entry> {
        let parent = parent_of(path);
        let mut inner = self.inner.lock().unwrap();
        let fresh = matches!(inner.expiry.get(parent), Some(exp) if *exp > Instant::now());
        if !fresh {
            inner.entries.remove(path);
            return None;
        }
        inner.entries.get(path).cloned()
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
                    created: None,
                    etag: e.etag.clone(),
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
        let child_keys: Vec<String> = entries
            .iter()
            .map(|e| format!("{prefix}{}", e.name))
            .collect();
        // Drop the subtree of any child present in the previous listing but gone
        // from this one, so a deleted child can't keep stat'ing as present.
        let vanished: Vec<String> = match inner.children.get(path) {
            Some(old) => {
                let fresh: std::collections::HashSet<&str> =
                    child_keys.iter().map(String::as_str).collect();
                old.iter()
                    .filter(|c| !fresh.contains(c.as_str()))
                    .cloned()
                    .collect()
            }
            None => Vec::new(),
        };
        for v in &vanished {
            purge_prefix(&mut inner, v);
        }
        for (e, full) in entries.iter().zip(&child_keys) {
            inner.entries.insert(
                full.clone(),
                Entry {
                    is_dir: matches!(e.kind, FileKind::Dir),
                    size: e.size,
                    mtime: e.mtime,
                    atime: e.atime,
                    ctime: e.ctime,
                    etag: e.etag.clone(),
                },
            );
        }
        inner.children.insert(path.to_string(), child_keys);
        inner
            .expiry
            .insert(path.to_string(), Instant::now() + self.ttl);
    }

    /// Drop `path` and its entire subtree (a direct-child sweep would leave
    /// grandchild entries/listings behind after a subtree `rmdir`/`rename`).
    fn invalidate_dir(&self, path: &str) {
        let mut inner = self.inner.lock().unwrap();
        purge_prefix(&mut inner, path);
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

/// Per-provider content-cache budget; over it, LRU-evict. Lossless — every read
/// revalidates, so a dropped entry just refetches.
const CONTENT_CACHE_BUDGET: u64 = 128 << 20;

/// Staleness backstop: Notion's minute-granular `last_edited_time` can miss a
/// same-minute edit, so a time-versioned entry refetches past this age.
const CONTENT_TTL: Duration = Duration::from_secs(300);

/// One cached object plus its LRU tick.
struct CacheEntry {
    version: Version,
    bytes: Arc<Vec<u8>>,
    seq: u64,
    fetched: Instant,
}

/// Content cache, LRU-bounded by total bytes. `order` (tick → path) keeps the
/// least-recently-used entry as its first key.
#[derive(Default)]
struct ContentCache {
    by_path: HashMap<String, CacheEntry>,
    order: BTreeMap<u64, String>,
    total: u64,
    tick: u64,
}

impl ContentCache {
    /// Mark `key` most-recently-used and return its bytes (caller ensured it exists).
    fn bump(&mut self, key: &str) -> Arc<Vec<u8>> {
        self.tick += 1;
        let seq = self.tick;
        let e = self.by_path.get_mut(key).expect("bump: entry present");
        let old = e.seq;
        e.seq = seq;
        let bytes = e.bytes.clone();
        self.order.remove(&old);
        self.order.insert(seq, key.to_string());
        bytes
    }

    /// Bytes for `key` iff its stored validator matches `version` (LRU-touched).
    fn get(&mut self, key: &str, version: &Version) -> Option<Arc<Vec<u8>>> {
        let hit = match self.by_path.get(key) {
            // A time-based version (Notion's minute-granular `last_edited_time`)
            // can miss a same-minute edit, so expire it past CONTENT_TTL; a strong
            // tag (S3 ETag) is exact and needs no backstop.
            Some(e) if &e.version == version => {
                !matches!(version, Version::Time(_)) || e.fetched.elapsed() < CONTENT_TTL
            }
            _ => false,
        };
        hit.then(|| self.bump(key))
    }

    /// Insert `key`, then evict least-recently-used entries until within `budget`.
    fn put(&mut self, key: &str, version: Version, bytes: Vec<u8>, budget: u64) {
        self.remove(key);
        self.tick += 1;
        let seq = self.tick;
        self.total += bytes.len() as u64;
        self.order.insert(seq, key.to_string());
        self.by_path.insert(
            key.to_string(),
            CacheEntry {
                version,
                bytes: Arc::new(bytes),
                seq,
                fetched: Instant::now(),
            },
        );
        while self.total > budget {
            let Some(victim) = self.order.values().next().cloned() else {
                break;
            };
            if victim == key {
                break; // never evict the just-inserted (MRU) entry
            }
            self.remove(&victim);
        }
    }

    /// Remove `key`'s entry if present, updating the byte total.
    fn remove(&mut self, key: &str) {
        if let Some(e) = self.by_path.remove(key) {
            self.order.remove(&e.seq);
            self.total -= e.bytes.len() as u64;
        }
    }
}

/// Wraps a provider [`Resource`] with an [`IndexCache`] for listings (short TTL)
/// plus a **validated** content cache: a read probes the object's cheap version
/// via `stat` and serves cached bytes only while it matches, so external edits
/// are refetched without a TTL wait. Mutations invalidate both.
pub struct CachedResource {
    inner: Arc<dyn Resource>,
    cache: IndexCache,
    /// Whole-object content, LRU-bounded ([`CONTENT_CACHE_BUDGET`]); freshness by
    /// revalidation, not a TTL.
    content: Mutex<ContentCache>,
}

impl CachedResource {
    pub fn new(inner: Arc<dyn Resource>) -> Self {
        Self {
            inner,
            cache: IndexCache::new(LISTING_TTL),
            content: Mutex::new(ContentCache::default()),
        }
    }

    /// Cached bytes for `key` iff the stored validator equals `version` (and is
    /// known) — the metadata dirty check; a changed backend version misses.
    fn content_get(&self, key: &str, version: &Version) -> Option<Arc<Vec<u8>>> {
        if !version.known() {
            return None;
        }
        self.content.lock().unwrap().get(key, version)
    }

    fn content_put(&self, key: &str, version: Version, bytes: Vec<u8>) {
        if version.known() && bytes.len() as u64 <= MAX_CONTENT_BYTES {
            self.content
                .lock()
                .unwrap()
                .put(key, version, bytes, CONTENT_CACHE_BUDGET);
        }
    }

    fn content_drop(&self, key: &str) {
        self.content.lock().unwrap().remove(key);
    }

    fn content_clear(&self) {
        *self.content.lock().unwrap() = ContentCache::default();
    }

    /// True length of a file the backend listed as size 0 (sizing it needs a
    /// render, e.g. Notion `page.json`): render once *through the content cache*
    /// so the size is known and a following read is a hit. Cheap on a cache hit;
    /// 0 if the render fails.
    async fn resolve_size(&self, path: &MountPath, version: &Version) -> u64 {
        if let Some(bytes) = self.content_get(path.as_str(), version) {
            return bytes.len() as u64;
        }
        match self.inner.read_bytes(path, None).await {
            Ok(bytes) => {
                let n = bytes.len() as u64;
                self.content_put(path.as_str(), version.clone(), bytes);
                n
            }
            Err(_) => 0,
        }
    }
}

/// A child path under `dir` (handles the root so the result has no `//`).
fn child_path(dir: &MountPath, name: &str) -> MountPath {
    if dir.is_root() {
        MountPath::new(format!("/{name}"))
    } else {
        MountPath::new(format!("{}/{}", dir.as_str(), name))
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
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        let key = path.as_str();
        // Validate the cheap version and serve cached bytes only while it matches.
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

    async fn read_bytes_pinned(
        &self,
        path: &MountPath,
        range: Option<Range<u64>>,
        stat: &FileStat,
    ) -> ResourceResult<Vec<u8>> {
        let key = path.as_str();
        // Validate against the version pinned when the read opened — no per-chunk
        // stat — so every chunk of one read comes from a single snapshot.
        let version = Version::of(stat);
        if let Some(full) = self.content_get(key, &version) {
            return Ok(slice(&full, &range));
        }
        let fetch_full = range.is_none() || stat.size == 0 || stat.size <= MAX_CONTENT_BYTES;
        if fetch_full {
            // Fetch pinned, not plain: on S3 this sends `If-Match(etag)`, so a
            // miss whose cached snapshot was evicted/replaced can't return newer
            // bytes under the old pin (which would tear an interleaved read and
            // mislabel the cache entry). A successful fetch matches the pin.
            let full = self.inner.read_bytes_pinned(path, None, stat).await?;
            let out = slice(&full, &range);
            self.content_put(key, version, full);
            Ok(out)
        } else {
            // Large uncached object: read the range pinned to `stat` — S3 sends
            // `If-Match(etag)` so a mid-stream change fails instead of tearing.
            self.inner.read_bytes_pinned(path, range, stat).await
        }
    }

    async fn write_bytes(&self, path: &MountPath, data: Vec<u8>) -> ResourceResult<()> {
        let r = self.inner.write_bytes(path, data).await;
        if r.is_ok() {
            self.cache.invalidate_parent(path.as_str());
            self.content_drop(path.as_str());
        }
        r
    }

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        if let Some(entries) = self.cache.list_dir_entries(path.as_str()) {
            return Ok(entries);
        }
        // Virtual `.search/` routing (only on providers with server-side
        // search): the root lists empty — a query dir materializes by being
        // asked for — and `.search/<query>` runs the provider's search. Results
        // are cached like any listing; deeper paths fall through to the
        // provider, which resolves them by id.
        if self.inner.supports_search()
            && let Some(query) = search_query(path.as_str())
        {
            let entries = if query.is_empty() {
                Vec::new()
            } else {
                self.inner.search(query).await?
            };
            self.cache.set_dir(path.as_str(), &entries);
            return Ok(entries);
        }
        // Incremental discovery for incomplete-listing providers (gmail): if we
        // just resolved a child its parent's (capped) listing didn't include,
        // drop the parent listing so the next readdir re-runs and folds it in
        // (the provider remembers visited entries). Checked before set_dir,
        // which would otherwise mark this path as listed.
        let discovered = !self.inner.listings_complete()
            && !path.is_root()
            && self.cache.is_listed(parent_of(path.as_str()))
            && self.cache.get(path.as_str()).is_none();
        let mut entries = self.inner.readdir(path).await?;
        // Eagerly size (and content-cache) files the backend listed as 0 because
        // sizing needs a render (Notion page.json): render once here so the entry
        // shows a real size AND a following read is a cache hit. This adds that
        // render to the listing's latency — the trade for correct sizes up front.
        for e in entries.iter_mut() {
            if matches!(e.kind, FileKind::File) && e.size == 0 {
                let child = child_path(path, &e.name);
                // Take the version from the provider's own stat, not the listing
                // DirEntry (which may omit mtime, e.g. Notion `page.json`): the
                // content cached here must be validated against the same token a
                // later read computes, or the read misses and re-renders.
                if let Ok(st) = self.inner.stat(&child).await {
                    e.size = self.resolve_size(&child, &Version::of(&st)).await;
                }
            }
        }
        self.cache.set_dir(path.as_str(), &entries);
        if discovered {
            self.cache.invalidate_dir(parent_of(path.as_str()));
        }
        Ok(entries)
    }

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
        let key = path.as_str();
        // The virtual `.search` root and any `.search/<query>` are dirs by
        // definition (when the provider searches at all) — nothing to probe.
        if self.inner.supports_search() && search_query(key).is_some() {
            return Ok(FileStat {
                kind: FileKind::Dir,
                ..Default::default()
            });
        }
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
                    // Keep the strong tag so a read opened off this cached stat
                    // can pin itself to the ETag (`If-Match`); dropping it here
                    // silently disabled the pin on the PROPFIND-then-GET path.
                    etag: e.etag.clone(),
                    ..Default::default()
                });
            }
            // A file cached with size 0 is ambiguous (providers report 0 for
            // sizes they don't cheaply know): resolve it below rather than trust
            // it, so reads/Content-Length aren't clamped to nothing.
            Some(_) => {}
            None => {
                // Negative cache: a fresh parent listing that lacks this path
                // proves it does not exist — skip the network probe. Only valid
                // when the provider's listings are complete; some return false
                // (e.g. Gmail, whose date index is TTL-cached, so a just-arrived
                // message may not be listed yet), so a missing child may still
                // exist and must be probed.
                if !path.is_root()
                    && self.inner.listings_complete()
                    && self.cache.is_listed(parent_of(key))
                {
                    return Err(ResourceError::NotFound);
                }
            }
        }
        let st = self.inner.stat(path).await?;
        // A file the backend can't cheaply size (render-on-read, e.g. Notion
        // page.json) reports 0 — resolve its real length via the content cache so
        // stat/HEAD/PROPFIND are correct and a following read is a hit.
        if matches!(st.kind, FileKind::File) && st.size == 0 {
            let version = Version::of(&st);
            let size = self.resolve_size(path, &version).await;
            return Ok(FileStat { size, ..st });
        }
        Ok(st)
    }

    async fn unlink(&self, path: &MountPath) -> ResourceResult<()> {
        let r = self.inner.unlink(path).await;
        if r.is_ok() {
            // The path itself may have been a directory; drop its listing too.
            self.cache.invalidate_dir(path.as_str());
            self.cache.invalidate_parent(path.as_str());
            self.content_drop(path.as_str());
        }
        r
    }

    async fn mkdir(&self, path: &MountPath) -> ResourceResult<()> {
        let r = self.inner.mkdir(path).await;
        if r.is_ok() {
            self.cache.invalidate_parent(path.as_str());
        }
        r
    }

    async fn rmdir(&self, path: &MountPath) -> ResourceResult<()> {
        let r = self.inner.rmdir(path).await;
        if r.is_ok() {
            self.cache.invalidate_dir(path.as_str());
            self.cache.invalidate_parent(path.as_str());
        }
        r
    }

    async fn rename(&self, from: &MountPath, to: &MountPath) -> ResourceResult<()> {
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

    async fn command(&self, name: &str, body: &[u8]) -> ResourceResult<Vec<u8>> {
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

    fn supports_search(&self) -> bool {
        self.inner.supports_search()
    }

    async fn search(&self, query: &str) -> ResourceResult<Vec<DirEntry>> {
        self.inner.search(query).await
    }

    fn listings_complete(&self) -> bool {
        self.inner.listings_complete()
    }
}

/// `Some(query)` when `path` is the virtual search root (`""`) or a
/// `.search/<query>` dir; `None` for anything deeper — those belong to the
/// provider (it resolves search hits by id).
fn search_query(path: &str) -> Option<&str> {
    let rest = path.strip_prefix('/')?;
    match rest.split_once('/') {
        None if rest == SEARCH_DIR => Some(""),
        Some((first, q)) if first == SEARCH_DIR && !q.is_empty() && !q.contains('/') => Some(q),
        _ => None,
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

/// Remove `p` and everything under `p/` from every map (entries/children/expiry).
fn purge_prefix(inner: &mut Inner, p: &str) {
    let sub = if p == "/" {
        "/".to_string()
    } else {
        format!("{p}/")
    };
    let stale = |k: &str| k == p || k.starts_with(sub.as_str());
    inner.entries.retain(|k, _| !stale(k));
    inner.children.retain(|k, _| !stale(k));
    inner.expiry.retain(|k, _| !stale(k));
}

#[cfg(test)]
mod tests {
    use super::*;

    // A pinned read stays on one snapshot for its whole duration, and a fresh
    // ranged read never serves stale cached bytes after an external edit.
    #[tokio::test]
    async fn pinned_read_is_one_snapshot_and_never_stale() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{Duration as D, UNIX_EPOCH};

        struct Obj {
            ver: AtomicU64,
            data: Mutex<Vec<u8>>,
        }
        #[async_trait]
        impl Resource for Obj {
            async fn read_bytes(&self, _p: &MountPath, range: Option<Range<u64>>) -> ResourceResult<Vec<u8>> {
                Ok(slice(self.data.lock().unwrap().as_slice(), &range))
            }
            async fn write_bytes(&self, _p: &MountPath, _d: Vec<u8>) -> ResourceResult<()> {
                Ok(())
            }
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<DirEntry>> {
                Ok(vec![])
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
                // Size 0 (validated by mtime) — like a render-on-read provider.
                Ok(FileStat {
                    kind: FileKind::File,
                    mtime: Some(UNIX_EPOCH + D::from_secs(self.ver.load(Ordering::SeqCst))),
                    ..Default::default()
                })
            }
        }

        let obj = Arc::new(Obj {
            ver: AtomicU64::new(1),
            data: Mutex::new(b"hello world".to_vec()),
        });
        let cached = CachedResource::new(obj.clone());
        let p = MountPath::new("/f.txt");

        // Read opens by stat'ing (v1); the first chunk caches that snapshot.
        let s1 = obj.stat(&p).await.unwrap();
        assert_eq!(cached.read_bytes_pinned(&p, Some(0..5), &s1).await.unwrap(), b"hello");

        // The object is edited behind the cache.
        *obj.data.lock().unwrap() = b"EDITED text".to_vec();
        obj.ver.store(2, Ordering::SeqCst);

        // A continuation chunk of the SAME read (pinned to v1) stays on v1 — no
        // torn response.
        assert_eq!(cached.read_bytes_pinned(&p, Some(6..11), &s1).await.unwrap(), b"world");

        // A fresh read stats again (v2); its ranged request must NOT serve the
        // stale v1 bytes.
        let s2 = obj.stat(&p).await.unwrap();
        assert_eq!(cached.read_bytes_pinned(&p, Some(6..11), &s2).await.unwrap(), b" text");
    }

    // A's pinned snapshot can be evicted/replaced by B's fresh read before A's
    // continuation chunk. On an etag backend (If-Match) the stale-pin refetch
    // must fail cleanly rather than return B's newer bytes under A's pin. (A
    // Time-only backend has no conditional GET, so this guarantee is etag-only.)
    #[tokio::test]
    async fn pinned_read_interleaved_with_refetch() {
        use std::sync::atomic::{AtomicU64, Ordering};

        struct Obj {
            ver: AtomicU64,
            data: Mutex<Vec<u8>>,
        }
        impl Obj {
            fn etag(&self) -> String {
                format!("\"v{}\"", self.ver.load(Ordering::SeqCst))
            }
        }
        #[async_trait]
        impl Resource for Obj {
            async fn read_bytes(
                &self,
                _p: &MountPath,
                range: Option<Range<u64>>,
            ) -> ResourceResult<Vec<u8>> {
                Ok(slice(self.data.lock().unwrap().as_slice(), &range))
            }
            async fn read_bytes_pinned(
                &self,
                p: &MountPath,
                range: Option<Range<u64>>,
                stat: &FileStat,
            ) -> ResourceResult<Vec<u8>> {
                // If-Match: a pin whose etag no longer matches current fails.
                if stat.etag.as_deref() != Some(self.etag().as_str()) {
                    return Err(ResourceError::Backend(anyhow::anyhow!(
                        "precondition failed"
                    )));
                }
                self.read_bytes(p, range).await
            }
            async fn write_bytes(&self, _p: &MountPath, _d: Vec<u8>) -> ResourceResult<()> {
                Ok(())
            }
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<DirEntry>> {
                Ok(vec![])
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
                Ok(FileStat {
                    kind: FileKind::File,
                    size: 11,
                    etag: Some(self.etag()),
                    ..Default::default()
                })
            }
        }

        let obj = Arc::new(Obj {
            ver: AtomicU64::new(1),
            data: Mutex::new(b"hello world".to_vec()),
        });
        let cached = CachedResource::new(obj.clone());
        let p = MountPath::new("/f.txt");

        // A opens (pins v1) and reads its first chunk.
        let s1 = obj.stat(&p).await.unwrap();
        assert_eq!(cached.read_bytes_pinned(&p, Some(0..5), &s1).await.unwrap(), b"hello");

        // External edit -> v2.
        *obj.data.lock().unwrap() = b"EDITED text".to_vec();
        obj.ver.store(2, Ordering::SeqCst);

        // B opens fresh (pins v2); its read refetches and replaces the cache
        // entry with v2 bytes.
        let s2 = obj.stat(&p).await.unwrap();
        assert_eq!(cached.read_bytes_pinned(&p, Some(0..5), &s2).await.unwrap(), b"EDITE");

        // A's continuation, still pinned to v1: cache holds v2 now, so it misses
        // and refetches under If-Match(v1) -> clean error, not a torn v2 read.
        let a2 = cached.read_bytes_pinned(&p, Some(6..11), &s1).await;
        assert!(a2.is_err(), "stale v1 pin must fail cleanly, not tear: {a2:?}");
    }

    // A stat served from the metadata cache (primed by readdir, i.e. the
    // PROPFIND-then-GET flow) must keep the provider's etag — that is the token
    // the pinned S3 read sends as If-Match.
    #[tokio::test]
    async fn cached_stat_preserves_etag_for_pinning() {
        struct S3Like;
        #[async_trait]
        impl Resource for S3Like {
            async fn read_bytes(
                &self,
                _p: &MountPath,
                range: Option<Range<u64>>,
            ) -> ResourceResult<Vec<u8>> {
                Ok(slice(b"0123456789", &range))
            }
            async fn write_bytes(&self, _p: &MountPath, _d: Vec<u8>) -> ResourceResult<()> {
                Ok(())
            }
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<DirEntry>> {
                Ok(vec![DirEntry {
                    name: "f.bin".into(),
                    kind: FileKind::File,
                    size: 10,
                    mtime: Some(std::time::UNIX_EPOCH),
                    atime: None,
                    ctime: None,
                    created: None,
                    etag: Some("\"abc123\"".into()),
                }])
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
                Ok(FileStat {
                    kind: FileKind::File,
                    size: 10,
                    mtime: Some(std::time::UNIX_EPOCH),
                    etag: Some("\"abc123\"".into()),
                    ..Default::default()
                })
            }
        }

        let cached = CachedResource::new(Arc::new(S3Like));
        // PROPFIND primes the metadata cache ...
        cached.readdir(&MountPath::new("/")).await.unwrap();
        // ... then GET opens the file; WorkspaceFs::open pins this stat.
        let pinned = cached.stat(&MountPath::new("/f.bin")).await.unwrap();
        assert_eq!(
            pinned.etag.as_deref(),
            Some("\"abc123\""),
            "etag dropped by the cache fast-path: If-Match pin is disabled"
        );
    }

    // The content cache evicts by total-byte budget, least-recently-used first,
    // and never drops the entry just inserted.
    #[test]
    fn content_cache_evicts_lru_over_budget() {
        let v = Version::Time(std::time::UNIX_EPOCH);
        let mut c = ContentCache::default();
        c.put("/a", v.clone(), vec![0u8; 4], 10);
        c.put("/b", v.clone(), vec![0u8; 4], 10);
        // Touch /a so /b is now the least-recently-used.
        assert!(c.get("/a", &v).is_some());
        // total would be 12 > 10 → evict the LRU (/b), keep the touched /a + new /c.
        c.put("/c", v.clone(), vec![0u8; 4], 10);
        assert!(c.get("/a", &v).is_some(), "recently used, kept");
        assert!(c.get("/c", &v).is_some(), "just inserted, kept");
        assert!(c.get("/b", &v).is_none(), "least-recently-used, evicted");
        assert!(c.total <= 10, "within budget");
    }

    // `.search` routing: on a provider with server-side search the virtual
    // root/query dirs exist and a query listing calls Resource::search (and is
    // cached); without support, nothing is synthesized.
    #[tokio::test]
    async fn search_dir_routes_to_provider_search() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Searchy {
            calls: AtomicUsize,
        }
        #[async_trait]
        impl Resource for Searchy {
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
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<DirEntry>> {
                Err(ResourceError::NotFound)
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
                Err(ResourceError::NotFound)
            }
            fn supports_search(&self) -> bool {
                true
            }
            async fn search(&self, q: &str) -> ResourceResult<Vec<DirEntry>> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![de(&format!("hit-{q}.json"), FileKind::File, 3)])
            }
        }
        let inner = Arc::new(Searchy {
            calls: AtomicUsize::new(0),
        });
        let cached = CachedResource::new(inner.clone());
        // The virtual root: a dir, listing empty.
        let root = MountPath::new("/.search");
        assert!(matches!(
            cached.stat(&root).await.unwrap().kind,
            FileKind::Dir
        ));
        assert!(cached.readdir(&root).await.unwrap().is_empty());
        // A query dir: stat'able, and its listing routes to search().
        let q = MountPath::new("/.search/foo bar");
        assert!(matches!(cached.stat(&q).await.unwrap().kind, FileKind::Dir));
        let hits = cached.readdir(&q).await.unwrap();
        assert_eq!(hits[0].name, "hit-foo bar.json");
        // Cached: a repeat listing and a child stat cost no further search.
        cached.readdir(&q).await.unwrap();
        let st = cached
            .stat(&MountPath::new("/.search/foo bar/hit-foo bar.json"))
            .await
            .unwrap();
        assert_eq!(st.size, 3);
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1, "one search only");
    }

    #[tokio::test]
    async fn search_dir_absent_without_provider_support() {
        struct NoSearch;
        #[async_trait]
        impl Resource for NoSearch {
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
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<DirEntry>> {
                Err(ResourceError::NotFound)
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
                Err(ResourceError::NotFound)
            }
        }
        let cached = CachedResource::new(Arc::new(NoSearch));
        // No synthesized dir: the provider's own answer (NotFound) stands.
        assert!(cached.stat(&MountPath::new("/.search")).await.is_err());
        assert!(
            cached
                .readdir(&MountPath::new("/.search/query"))
                .await
                .is_err()
        );
    }

    fn de(name: &str, kind: FileKind, size: u64) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            kind,
            size,
            mtime: None,
            atime: None,
            ctime: None,
            created: None,
            etag: None,
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
                created: None,
                etag: None,
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

    // get() honors the owning listing's TTL: an expired entry is dropped and
    // reported absent, not served via a no-TTL fast path.
    #[test]
    fn expired_entry_is_not_served_as_fresh() {
        let c = IndexCache::new(Duration::from_millis(0));
        c.set_dir("/", &[de("gone.txt", FileKind::File, 10)]);
        assert!(!c.is_listed("/"));
        assert!(c.get("/gone.txt").is_none());
    }

    // Re-listing a directory drops children that disappeared, so a deleted
    // object stops stat'ing as present even while the listing is fresh.
    #[test]
    fn relisting_a_dir_drops_vanished_children() {
        let c = IndexCache::new(Duration::from_secs(600));
        c.set_dir("/d", &[de("gone.txt", FileKind::File, 3)]);
        c.set_dir("/d", &[]); // gone.txt deleted upstream
        assert!(c.get("/d/gone.txt").is_none());
    }

    // Invalidating a directory drops its whole subtree, not just direct children.
    #[test]
    fn subtree_invalidation_drops_grandchildren() {
        let c = IndexCache::new(Duration::from_secs(600));
        c.set_dir("/a/b", &[de("sub", FileKind::Dir, 0)]);
        c.set_dir("/a/b/sub", &[de("d.txt", FileKind::File, 7)]);
        c.invalidate_dir("/a/b");
        assert!(c.get("/a/b/sub/d.txt").is_none());
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
                _p: &MountPath,
                range: Option<Range<u64>>,
            ) -> ResourceResult<Vec<u8>> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                Ok(slice(self.data.lock().unwrap().as_slice(), &range))
            }
            async fn write_bytes(&self, _p: &MountPath, data: Vec<u8>) -> ResourceResult<()> {
                *self.data.lock().unwrap() = data;
                Ok(())
            }
            async fn readdir(&self, _p: &MountPath) -> ResourceResult<Vec<DirEntry>> {
                Ok(vec![])
            }
            async fn stat(&self, _p: &MountPath) -> ResourceResult<FileStat> {
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
        let p = MountPath::new("/f.txt");

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
