//! Content-hash knowledge indexing (the current approach, ported to the
//! `workspace` crate + `FsHook`).
//!
//! Membership is a set of `*.ref` files under `/files/knowledge`, each holding
//! the unified-tree path of a target. Resync/reconcile against
//! [`agent_k::knowledge_base::Store`]: enumerate targets, read + `ingest_many` in
//! byte-bounded batches (document ids are a content hash, so unchanged content is
//! a no-op), then purge documents no longer referenced. A per-workspace in-memory
//! memo protects a still-referenced-but-unreadable target's document from a
//! transient-failure purge (see `build_keepset`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_k::knowledge_base::{FileType, PdfEngine, SharedStore, Store};
use dashmap::DashMap;
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use ::workspace::{FsError, FsResult, OpenOptions, WorkspaceFs};

use super::workspace::{build_workspace_vfs, workspace_fs};

/// Unified-tree directory holding the index references. Local files live under
/// the `/files` mount, so knowledge refs are `/files/knowledge/*.ref`.
pub const KNOWLEDGE_DIR: &str = "/files/knowledge";

/// Suffix marking a reference file under [`KNOWLEDGE_DIR`].
pub const REF_SUFFIX: &str = ".ref";

/// Resident-byte budget per ingest batch: bytes are dropped between batches so
/// peak memory stays bounded. A file over budget forms its own batch (bounded
/// instead by [`MAX_FILE_BYTES`]). Near `Store`'s ~128 MB `IndexWriter`.
const BATCH_BUDGET_BYTES: usize = 128 * 1024 * 1024;

/// Per-file cap. A file can't be streamed (read whole to hash/translate), so an
/// arbitrarily large one could OOM; files over this are skipped.
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

const READ_CHUNK: usize = 256 * 1024;

/// The on-disk body of a `*.ref` file: the unified-tree path of its target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeRef {
    pub path: String,
}

impl KnowledgeRef {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("serialise KnowledgeRef")
    }
}

/// Outcome of the most recent reconcile for a single referenced target.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Indexed,
    ReadFailed,
    IngestFailed,
}

/// Per-target state remembered between reconciles (in-memory only; rebuilt on the
/// next reconcile after a restart).
#[derive(Debug, Clone)]
struct TargetState {
    /// Content-hash id of the doc last indexed for this target, carried across
    /// failed cycles so a still-referenced-but-unreadable target survives purge.
    last_indexed_id: Option<Uuid>,
    #[allow(dead_code)]
    outcome: Outcome,
}

/// Indexes a workspace's referenced knowledge into its [`Store`]. Cheap to clone,
/// so one handle serves the fs write-trigger hook, the router, and the loop.
#[derive(Clone)]
pub struct Resyncer {
    db: SqlitePool,
    data_root: PathBuf,
    stores: Arc<DashMap<Uuid, SharedStore>>,
    locks: Arc<DashMap<Uuid, Arc<Mutex<()>>>>,
    /// Per-workspace, per-target state (path → state); in-memory only.
    refmaps: Arc<DashMap<Uuid, HashMap<String, TargetState>>>,
}

impl Resyncer {
    pub fn new(db: SqlitePool, data_root: PathBuf) -> Self {
        Self {
            db,
            data_root,
            stores: Arc::new(DashMap::new()),
            locks: Arc::new(DashMap::new()),
            refmaps: Arc::new(DashMap::new()),
        }
    }

    fn store_root(&self, wid: Uuid) -> PathBuf {
        self.data_root
            .join("workspaces")
            .join(wid.to_string())
            .join("knowledge-index")
    }

    fn store_for(&self, wid: Uuid) -> anyhow::Result<SharedStore> {
        use dashmap::mapref::entry::Entry;
        match self.stores.entry(wid) {
            Entry::Occupied(o) => Ok(o.get().clone()),
            Entry::Vacant(v) => {
                let store = Store::new(self.store_root(wid))?;
                let shared = Arc::new(RwLock::new(store));
                v.insert(shared.clone());
                Ok(shared)
            }
        }
    }

    fn lock_for(&self, wid: Uuid) -> Arc<Mutex<()>> {
        self.locks
            .entry(wid)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Drop the cached [`Store`], lock, and per-target memo for a torn-down
    /// workspace. Takes the per-workspace lock first so it can't interleave with
    /// an in-flight reconcile whose tail would resurrect the memo.
    pub async fn forget(&self, wid: Uuid) {
        let lock = self.lock_for(wid);
        let _guard = lock.lock().await;
        self.stores.remove(&wid);
        self.locks.remove(&wid);
        self.refmaps.remove(&wid);
    }

    /// Spawn a background resync, logging any failure.
    pub fn spawn_resync(&self, wid: Uuid) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.reconcile(wid).await {
                tracing::warn!(%wid, "knowledge resync failed: {e:#}");
            }
        });
    }

    /// Reconcile `wid`'s [`Store`] with its `/files/knowledge` references. Ingest
    /// referenced targets in byte-bounded batches, then purge documents whose ref
    /// is gone. Serialized per workspace.
    pub async fn reconcile(&self, wid: Uuid) -> anyhow::Result<()> {
        let lock = self.lock_for(wid);
        let _guard = lock.lock().await;

        let config = build_workspace_vfs(&self.db, wid).await?;
        let fs = workspace_fs(&self.data_root, wid, config)?;

        let targets = collect_targets(&fs, MAX_FILE_BYTES).await?;
        let member_count = targets.len();
        if member_count == 0 && !self.store_root(wid).join("index").exists() {
            return Ok(());
        }

        let store = self.store_for(wid)?;
        let mut store = store.write().await;

        let prev = self.refmaps.get(&wid).map(|m| m.clone()).unwrap_or_default();

        // Read each target (remembering its content-hash id, or that it failed)
        // and ingest readable bytes in byte-bounded batches.
        let mut succeeded: HashSet<Uuid> = HashSet::new();
        let mut reads: Vec<(String, Option<Uuid>)> = Vec::new();
        let mut batch: Vec<(Vec<u8>, FileType)> = Vec::new();
        let mut batch_bytes = 0usize;
        for (path, ft) in targets {
            match read_all(&fs, &path).await {
                Ok(bytes) => {
                    let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &bytes);
                    reads.push((path, Some(id)));
                    if !batch.is_empty() && batch_bytes + bytes.len() > BATCH_BUDGET_BYTES {
                        ingest_batch(&mut store, std::mem::take(&mut batch), wid, &mut succeeded)
                            .await?;
                        batch_bytes = 0;
                    }
                    batch_bytes += bytes.len();
                    batch.push((bytes, ft));
                }
                Err(e) => {
                    tracing::warn!(%wid, "unreadable target {path}: {e:?}");
                    reads.push((path, None));
                }
            }
        }
        if !batch.is_empty() {
            ingest_batch(&mut store, batch, wid, &mut succeeded).await?;
        }

        let (next, protected) = build_keepset(&prev, &reads, &succeeded);
        self.refmaps.insert(wid, next);

        // Purge only docs whose ref is gone — never docs that merely failed to
        // read/ingest this cycle (protected via the memo above).
        let stale: Vec<Uuid> = store
            .list(false, 0, u32::MAX)?
            .into_iter()
            .filter_map(|d| Uuid::parse_str(&d.id).ok())
            .filter(|id| !protected.contains(id))
            .collect();
        if !stale.is_empty() {
            let purged = store.purge_many(stale);
            for f in &purged.failed {
                tracing::warn!(%wid, id = %f.id, "knowledge purge failed: {}", f.error);
            }
        }
        store.compact()?;
        Ok(())
    }
}

/// Build the new per-target memo and the purge keep-set from this cycle's scan
/// (`reads`: path → content-hash id read, or `None` on read failure) and the ids
/// `succeeded` confirms are in the store. A still-referenced target that fails to
/// read is protected via its remembered `last_indexed_id`; a removed ref is
/// absent from `reads`, so its id isn't kept and is purged.
fn build_keepset(
    prev: &HashMap<String, TargetState>,
    reads: &[(String, Option<Uuid>)],
    succeeded: &HashSet<Uuid>,
) -> (HashMap<String, TargetState>, HashSet<Uuid>) {
    let mut next = HashMap::new();
    let mut protected = HashSet::new();
    for (path, read_id) in reads {
        let prior_id = prev.get(path).and_then(|s| s.last_indexed_id);
        let state = match read_id {
            Some(id) if succeeded.contains(id) => {
                protected.insert(*id);
                TargetState {
                    last_indexed_id: Some(*id),
                    outcome: Outcome::Indexed,
                }
            }
            Some(_) => {
                if let Some(id) = prior_id {
                    protected.insert(id);
                }
                TargetState {
                    last_indexed_id: prior_id,
                    outcome: Outcome::IngestFailed,
                }
            }
            None => {
                if let Some(id) = prior_id {
                    protected.insert(id);
                }
                TargetState {
                    last_indexed_id: prior_id,
                    outcome: Outcome::ReadFailed,
                }
            }
        };
        next.insert(path.clone(), state);
    }
    (next, protected)
}

/// Ingest one batch, folding confirmed ids into `succeeded`; per-item failures
/// are logged, not fatal.
async fn ingest_batch(
    store: &mut Store,
    batch: Vec<(Vec<u8>, FileType)>,
    wid: Uuid,
    succeeded: &mut HashSet<Uuid>,
) -> anyhow::Result<()> {
    let result = store.ingest_many(batch, PdfEngine::default()).await?;
    for f in &result.failed {
        tracing::warn!(%wid, "knowledge ingest failed: {}", f.error);
    }
    succeeded.extend(result.succeeded);
    Ok(())
}

/// Read an entire file into memory via the unified tree.
async fn read_all(fs: &WorkspaceFs, path: &str) -> FsResult<Vec<u8>> {
    let mut file = fs
        .open(
            path,
            OpenOptions {
                read: true,
                ..Default::default()
            },
        )
        .await?;
    let mut out = Vec::new();
    loop {
        let chunk = file.read_bytes(READ_CHUNK).await?;
        if chunk.is_empty() {
            break;
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Walk `/files/knowledge`, resolve each `*.ref` to its target(s), and return the
/// indexable files' paths + types — without reading their bytes.
async fn collect_targets(
    fs: &WorkspaceFs,
    max_file_bytes: u64,
) -> anyhow::Result<Vec<(String, FileType)>> {
    let mut out = Vec::new();
    let mut stream = match fs.read_dir(KNOWLEDGE_DIR).await {
        Ok(s) => s,
        Err(FsError::NotFound) => return Ok(out),
        Err(e) => anyhow::bail!("read {KNOWLEDGE_DIR}: {e:?}"),
    };
    while let Some(entry) = stream.next().await {
        let entry = entry.map_err(|e| anyhow::anyhow!("read {KNOWLEDGE_DIR}: {e:?}"))?;
        let name = String::from_utf8_lossy(&entry.name()).into_owned();
        if !name.ends_with(REF_SUFFIX) {
            continue;
        }
        let ref_path = format!("{KNOWLEDGE_DIR}/{name}");
        let bytes = match read_all(fs, &ref_path).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("skipping unreadable reference {ref_path}: {e:?}");
                continue;
            }
        };
        let target = match serde_json::from_slice::<KnowledgeRef>(&bytes) {
            Ok(r) => r.path,
            Err(e) => {
                tracing::warn!("skipping malformed reference {ref_path}: {e}");
                continue;
            }
        };
        if !is_safe_target(&target) || is_under_knowledge(&target) {
            tracing::warn!("skipping unsafe/self reference target {target}");
            continue;
        }
        add_target(fs, &target, max_file_bytes, &mut out).await?;
    }
    // A directory ref overlapping a file ref (or two refs to the same target) can
    // enumerate the same path twice; keep the first so a path is indexed once.
    let mut seen = HashSet::new();
    out.retain(|(path, _)| seen.insert(path.clone()));
    Ok(out)
}

/// Enumerate a target into `out`: a file contributes itself, a directory its
/// indexable descendants. Oversized and missing targets are skipped.
async fn add_target(
    fs: &WorkspaceFs,
    target: &str,
    max_file_bytes: u64,
    out: &mut Vec<(String, FileType)>,
) -> anyhow::Result<()> {
    let meta = match fs.metadata(target).await {
        Ok(m) => m,
        Err(FsError::NotFound | FsError::Forbidden) => {
            tracing::warn!("skipping unresolvable target {target}");
            return Ok(());
        }
        Err(e) => anyhow::bail!("stat {target}: {e:?}"),
    };

    if !meta.is_dir() {
        if let Some(ft) = FileType::from_path(Path::new(target)) {
            if meta.len > max_file_bytes {
                tracing::warn!("skipping oversized target {target} ({} bytes)", meta.len);
            } else {
                out.push((target.to_string(), ft));
            }
        }
        return Ok(());
    }

    let mut stack = vec![target.trim_end_matches('/').to_string()];
    while let Some(dir) = stack.pop() {
        let mut stream = fs
            .read_dir(&dir)
            .await
            .map_err(|e| anyhow::anyhow!("read {dir}: {e:?}"))?;
        while let Some(entry) = stream.next().await {
            let entry = entry.map_err(|e| anyhow::anyhow!("read {dir}: {e:?}"))?;
            let name = String::from_utf8_lossy(&entry.name()).into_owned();
            let child = format!("{dir}/{name}");
            let is_dir = entry.metadata().map(|s| s.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push(child);
            } else if let Some(ft) = FileType::from_path(Path::new(&name)) {
                let len = entry.metadata().map(|s| s.len).unwrap_or(0);
                if len > max_file_bytes {
                    tracing::warn!("skipping oversized target {child} ({len} bytes)");
                } else {
                    out.push((child, ft));
                }
            }
        }
    }
    Ok(())
}

/// A reference target must be an absolute unified-tree path of normal segments.
fn is_safe_target(path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    let mut any = false;
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        any = true;
        if seg == "." || seg == ".." {
            return false;
        }
    }
    any
}

/// True when `path` is `/files/knowledge` or lives under it.
fn is_under_knowledge(path: &str) -> bool {
    let p = path.trim_start_matches('/');
    p == "files/knowledge" || p.starts_with("files/knowledge/")
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;

    use ::workspace::{FsConfig, WorkspaceFs};
    use uuid::Uuid;

    use super::{
        KnowledgeRef, Outcome, TargetState, build_keepset, collect_targets, is_safe_target,
    };

    fn indexed(id: Uuid) -> TargetState {
        TargetState {
            last_indexed_id: Some(id),
            outcome: Outcome::Indexed,
        }
    }

    async fn write(root: &Path, rel: &str, body: &[u8]) {
        let p = root.join(rel);
        tokio::fs::create_dir_all(p.parent().unwrap()).await.unwrap();
        tokio::fs::write(p, body).await.unwrap();
    }

    async fn write_ref(root: &Path, name: &str, target: &str) {
        let body = KnowledgeRef {
            path: target.into(),
        }
        .to_bytes();
        write(root, &format!("knowledge/{name}"), &body).await;
    }

    /// A still-referenced target that fails to read keeps its document (protected
    /// via its prior id) rather than being purged on a transient blip.
    #[test]
    fn transient_read_failure_protects_prior_doc() {
        let id = Uuid::from_u128(1);
        let mut prev = HashMap::new();
        prev.insert("/files/a.md".to_string(), indexed(id));
        let reads = vec![("/files/a.md".to_string(), None)];
        let (_next, protected) = build_keepset(&prev, &reads, &HashSet::new());
        assert!(protected.contains(&id));
    }

    /// A removed ref (absent from this cycle's reads) is not protected → purged.
    #[test]
    fn removed_ref_not_protected() {
        let id = Uuid::from_u128(2);
        let mut prev = HashMap::new();
        prev.insert("/files/gone.md".to_string(), indexed(id));
        let (_next, protected) = build_keepset(&prev, &[], &HashSet::new());
        assert!(!protected.contains(&id));
    }

    #[tokio::test]
    async fn collects_file_and_dir_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let files = tmp.path();
        write(files, "docs/a.md", b"AAA").await;
        write(files, "docs/sub/b.md", b"BBB").await;
        write(files, "docs/sub/ignore.bin", b"nope").await;
        write_ref(files, "a.ref", "/files/docs/a.md").await;
        write_ref(files, "dir.ref", "/files/docs/sub").await;
        write_ref(files, "missing.ref", "/files/docs/gone.md").await;
        write_ref(files, "self.ref", "/files/knowledge/a.ref").await;

        let fs = WorkspaceFs::from_config(FsConfig {
            local_root: Some(files.to_path_buf()),
            mounts: vec![],
        })
        .unwrap();
        let mut paths: Vec<String> = collect_targets(&fs, u64::MAX)
            .await
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            vec!["/files/docs/a.md".to_string(), "/files/docs/sub/b.md".to_string()]
        );
    }

    #[test]
    fn safe_target_rules() {
        assert!(is_safe_target("/files/a.md"));
        assert!(!is_safe_target("a.md"));
        assert!(!is_safe_target("/a/../b"));
        assert!(!is_safe_target("/"));
    }
}
