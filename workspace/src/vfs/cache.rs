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

/// Default listing TTL (600s).
const DEFAULT_TTL: Duration = Duration::from_secs(600);

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
        inner.entries.get(path).copied()
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
        let child_keys: Vec<String> =
            entries.iter().map(|e| format!("{prefix}{}", e.name)).collect();
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

/// Wraps a provider [`Resource`] with an [`IndexCache`]: `readdir` fills the
/// cache, `stat` serves from it (and negative-caches misses), mutations
/// invalidate. Read/write payloads are delegated unchanged.
pub struct CachedResource {
    inner: Arc<dyn Resource>,
    cache: IndexCache,
}

impl CachedResource {
    pub fn new(inner: Arc<dyn Resource>) -> Self {
        Self {
            inner,
            cache: IndexCache::new(DEFAULT_TTL),
        }
    }
}

#[async_trait]
impl Resource for CachedResource {
    async fn read_bytes(&self, path: &VPath, range: Option<Range<u64>>) -> VfsResult<Vec<u8>> {
        self.inner.read_bytes(path, range).await
    }

    async fn write_bytes(&self, path: &VPath, data: Vec<u8>) -> VfsResult<()> {
        let r = self.inner.write_bytes(path, data).await;
        if r.is_ok() {
            self.cache.invalidate_parent(path.as_str());
        }
        r
    }

    async fn readdir(&self, path: &VPath) -> VfsResult<Vec<DirEntry>> {
        if let Some(entries) = self.cache.list_dir_entries(path.as_str()) {
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
        let entries = self.inner.readdir(path).await?;
        self.cache.set_dir(path.as_str(), &entries);
        if discovered {
            self.cache.invalidate_dir(parent_of(path.as_str()));
        }
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
                // proves it does not exist — skip the network probe. Only valid
                // when the provider's listings are complete; gmail caps them, so
                // a missing child may still exist and must be probed.
                if !path.is_root()
                    && self.inner.listings_complete()
                    && self.cache.is_listed(parent_of(key))
                {
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
        }
        r
    }

    async fn command(&self, name: &str, body: &[u8]) -> VfsResult<Vec<u8>> {
        let r = self.inner.command(name, body).await;
        // A domain write (e.g. Notion page-create) may change listings anywhere
        // in the mount; conservatively drop the whole index.
        if r.is_ok() {
            self.cache.clear();
        }
        r
    }

    fn prompt(&self) -> &str {
        self.inner.prompt()
    }

    fn listings_complete(&self) -> bool {
        self.inner.listings_complete()
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
    let sub = if p == "/" { "/".to_string() } else { format!("{p}/") };
    let stale = |k: &str| k == p || k.starts_with(sub.as_str());
    inner.entries.retain(|k, _| !stale(k));
    inner.children.retain(|k, _| !stale(k));
    inner.expiry.retain(|k, _| !stale(k));
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
}
