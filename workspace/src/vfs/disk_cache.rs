//! Generic host-side persistent cache for provider mounts.
//!
//! Providers whose objects are **immutable once fetched** (a Gmail message
//! body, a Slack message, a GDrive file revision) can share one fetch across
//! every session mounting the same account — the per-session in-memory caches
//! stay as a thin first tier on top. Two kinds of entries:
//!
//! - **blobs**: immutable byte objects keyed by an id (`blobs/<key>`).
//!   No TTL — immutability means age can't make them wrong; the provider
//!   removes a blob when it mutates it away (e.g. trash).
//! - **snapshots**: whole serialized values (e.g. a label's date→ids index)
//!   stamped with a wall-clock `built_at` (`snapshots/<hash>.json`), so a new
//!   process can adopt them with their *remaining* freshness. Read/written
//!   wholesale — which is why this is files, not a database.
//!
//! Each provider instance opens its own root, conventionally
//! `<cache_root>/<provider>/<account-key>/` where `account-key` is a
//! [`stable_hash`] of a credential (never the credential itself).
//!
//! Concurrency: writes go to a temp file in the same dir and `rename` into
//! place, so concurrent sessions can't tear a file — and for immutable blobs
//! last-writer-wins is harmless. Eviction races (two processes deleting the
//! same victim) are tolerated: a missing victim is a no-op.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// On-disk envelope of one snapshot.
#[derive(Serialize, Deserialize)]
struct Snapshot<T> {
    /// Wall-clock build time (epoch ms) — wall clock, not `Instant`, so the
    /// snapshot's remaining freshness survives a process restart.
    built_at_ms: u64,
    /// The snapshot's logical name (informational; the filename is a hash).
    name: String,
    data: T,
}

/// Stable hash for filenames/account keys derived from arbitrary strings
/// (snapshot names and credentials can contain path-hostile chars). Not
/// security-sensitive — a collision or an across-versions change just means a
/// cold cache entry.
pub(crate) fn stable_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// See the module docs. One instance per provider account.
pub(crate) struct DiskCache {
    blobs: PathBuf,
    snaps: PathBuf,
    /// Total bytes under `blobs/`, scanned lazily on the first write (the scan
    /// walks the dir once; reads never need it). Guards the eviction budget.
    total: tokio::sync::Mutex<Option<u64>>,
    budget: u64,
}

impl DiskCache {
    /// Open (creating dirs) the cache rooted at `root`, evicting blobs past
    /// `budget` bytes. `None` if the dirs can't be created — the caller then
    /// just runs without a disk tier.
    pub(crate) fn open(root: &Path, budget: u64) -> Option<Self> {
        let blobs = root.join("blobs");
        let snaps = root.join("snapshots");
        for d in [&blobs, &snaps] {
            if let Err(e) = std::fs::create_dir_all(d) {
                tracing::warn!("disk cache disabled ({}): {e}", d.display());
                return None;
            }
        }
        Some(Self {
            blobs,
            snaps,
            total: tokio::sync::Mutex::new(None),
            budget,
        })
    }

    /// Path of a blob's cache file. Keys are typically provider ids (URL-safe
    /// alphanumerics), but sanitize defensively — they came out of API JSON.
    fn blob_path(&self, key: &str) -> PathBuf {
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.blobs.join(safe)
    }

    fn snap_path(&self, name: &str) -> PathBuf {
        self.snaps.join(format!("{:016x}.json", stable_hash(name)))
    }

    /// Write `bytes` to `path` atomically (same-dir temp + rename).
    async fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let tmp = path.with_extension(format!(
            "tmp.{}.{}",
            std::process::id(),
            fastrand::u32(..)
        ));
        tokio::fs::write(&tmp, bytes).await?;
        match tokio::fs::rename(&tmp, path).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                Err(e)
            }
        }
    }

    /// A blob's bytes, if present. Touches the file's mtime so eviction
    /// (oldest-mtime-first) approximates LRU rather than FIFO.
    pub(crate) async fn get_blob(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.blob_path(key);
        let bytes = tokio::fs::read(&path).await.ok()?;
        // Best-effort LRU touch; failure only skews eviction order.
        let _ = std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(SystemTime::now()));
        Some(bytes)
    }

    /// Persist one blob and evict back under budget if needed. A key that is
    /// already present is left as-is (blobs are immutable).
    pub(crate) async fn put_blob(&self, key: &str, bytes: &[u8]) {
        let path = self.blob_path(key);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return;
        }
        let len = bytes.len() as u64;
        if Self::write_atomic(&path, bytes).await.is_err() {
            return;
        }
        let mut total = self.total.lock().await;
        let t = match total.as_mut() {
            Some(t) => {
                *t += len;
                *t
            }
            None => {
                let scanned = self.scan_total().await;
                *total = Some(scanned);
                scanned
            }
        };
        if t > self.budget {
            let freed = self.evict(t - self.budget, &path).await;
            if let Some(t) = total.as_mut() {
                *t = t.saturating_sub(freed);
            }
        }
    }

    pub(crate) async fn remove_blob(&self, key: &str) {
        let path = self.blob_path(key);
        if let Ok(meta) = tokio::fs::metadata(&path).await
            && tokio::fs::remove_file(&path).await.is_ok()
            && let Some(t) = self.total.lock().await.as_mut()
        {
            *t = t.saturating_sub(meta.len());
        }
    }

    /// The named snapshot, as `(age, value)` — the caller applies its own TTL
    /// to `age`.
    pub(crate) async fn get_snapshot<T: DeserializeOwned>(
        &self,
        name: &str,
    ) -> Option<(Duration, T)> {
        let bytes = tokio::fs::read(self.snap_path(name)).await.ok()?;
        let snap: Snapshot<T> = serde_json::from_slice(&bytes).ok()?;
        let built = SystemTime::UNIX_EPOCH + Duration::from_millis(snap.built_at_ms);
        let age = SystemTime::now().duration_since(built).ok()?;
        Some((age, snap.data))
    }

    pub(crate) async fn put_snapshot<T: Serialize>(&self, name: &str, value: &T) {
        let built_at_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let snap = Snapshot {
            built_at_ms,
            name: name.to_string(),
            data: value,
        };
        if let Ok(bytes) = serde_json::to_vec(&snap) {
            let _ = Self::write_atomic(&self.snap_path(name), &bytes).await;
        }
    }

    /// Drop every snapshot (a mutation made them all potentially stale;
    /// they're cheap to rebuild from the API).
    pub(crate) async fn clear_snapshots(&self) {
        if let Ok(mut rd) = tokio::fs::read_dir(&self.snaps).await {
            while let Ok(Some(e)) = rd.next_entry().await {
                let _ = tokio::fs::remove_file(e.path()).await;
            }
        }
    }

    /// Sum of file sizes under `blobs/`.
    async fn scan_total(&self) -> u64 {
        let mut sum = 0u64;
        if let Ok(mut rd) = tokio::fs::read_dir(&self.blobs).await {
            while let Ok(Some(e)) = rd.next_entry().await {
                if let Ok(m) = e.metadata().await {
                    sum += m.len();
                }
            }
        }
        sum
    }

    /// Delete oldest-mtime blobs (never `keep`) until at least `need` bytes are
    /// freed; returns the bytes actually freed. Walks the dir — eviction only
    /// runs when the budget (GBs) is crossed, so the walk is rare.
    async fn evict(&self, need: u64, keep: &Path) -> u64 {
        let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        if let Ok(mut rd) = tokio::fs::read_dir(&self.blobs).await {
            while let Ok(Some(e)) = rd.next_entry().await {
                let path = e.path();
                if path == keep {
                    continue;
                }
                if let Ok(m) = e.metadata().await {
                    let mtime = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    files.push((mtime, m.len(), path));
                }
            }
        }
        files.sort_by_key(|(t, _, _)| *t);
        let mut freed = 0u64;
        for (_, len, path) in files {
            if freed >= need {
                break;
            }
            if tokio::fs::remove_file(&path).await.is_ok() {
                freed += len;
            }
        }
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn blob_roundtrip_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let c = DiskCache::open(tmp.path(), u64::MAX).unwrap();
        assert!(c.get_blob("m1").await.is_none(), "cold miss");
        c.put_blob("m1", b"hello").await;
        assert_eq!(c.get_blob("m1").await.unwrap(), b"hello", "hit after put");
        c.remove_blob("m1").await;
        assert!(c.get_blob("m1").await.is_none(), "gone after remove");
    }

    #[tokio::test]
    async fn evicts_oldest_when_over_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let payload = vec![0u8; 4096];
        // Budget fits ~2 of the 3 payloads.
        let c = DiskCache::open(tmp.path(), payload.len() as u64 * 2 + 10).unwrap();
        c.put_blob("a", &payload).await;
        // Backdate "a" so it is strictly the oldest.
        let old = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(c.blob_path("a"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        c.put_blob("b", &payload).await;
        c.put_blob("c", &payload).await; // over budget → oldest ("a") evicted
        assert!(c.get_blob("a").await.is_none(), "oldest evicted");
        assert!(c.get_blob("b").await.is_some());
        assert!(c.get_blob("c").await.is_some(), "just-written kept");
    }

    #[tokio::test]
    async fn snapshot_roundtrip_reports_age() {
        let tmp = tempfile::tempdir().unwrap();
        let c = DiskCache::open(tmp.path(), u64::MAX).unwrap();
        type Index = BTreeMap<String, Vec<String>>;
        assert!(c.get_snapshot::<Index>("INBOX").await.is_none(), "cold miss");
        let mut idx: Index = BTreeMap::new();
        idx.insert("2026-07-01".into(), vec!["m1".into(), "m2".into()]);
        c.put_snapshot("INBOX", &idx).await;
        let (age, loaded) = c.get_snapshot::<Index>("INBOX").await.unwrap();
        assert_eq!(loaded, idx);
        assert!(age < Duration::from_secs(5), "freshly built: {age:?}");
        // Path-hostile names get hashed filenames.
        c.put_snapshot("受信/箱 a?b", &idx).await;
        assert!(c.get_snapshot::<Index>("受信/箱 a?b").await.is_some());
        // clear_snapshots drops them all.
        c.clear_snapshots().await;
        assert!(c.get_snapshot::<Index>("INBOX").await.is_none());
    }

    #[tokio::test]
    async fn keys_are_sanitized_into_filenames() {
        let tmp = tempfile::tempdir().unwrap();
        let c = DiskCache::open(tmp.path(), u64::MAX).unwrap();
        c.put_blob("../../etc/passwd", b"x").await;
        // Stored under a sanitized name inside blobs/, not outside the cache.
        assert!(c.get_blob("../../etc/passwd").await.is_some());
        assert!(!tmp.path().join("../../etc/passwd").exists());
    }
}
