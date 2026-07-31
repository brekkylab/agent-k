//! Content-hash knowledge indexing over the workspace's cortex
//! [`Workspace`](cortex::Workspace).
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
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use agent_k::knowledge_base::{FileType, PdfEngine, SharedStore, Store};
use cortex::{CortexError, DirentKind, FileExt, Mountable, OpenOptions, Workspace};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{Mutex, OnceCell, RwLock};
use uuid::Uuid;

use super::workspace::{build_workspace_vfs, cortex_workspace_spec};

/// A workspace-relative path (leading slash, e.g. `/files/knowledge/a.ref`) as a
/// cortex mount path (no leading slash — cortex treats a request path as relative
/// to the workspace root). Knowledge paths keep the leading-slash form internally
/// (ref-file targets are stored that way; [`is_safe_target`] requires it).
fn cx(path: &str) -> PathBuf {
    PathBuf::from(path.trim_start_matches('/'))
}

/// Unified-tree directory holding the index references. Local files live under
/// the `/files` mount, so knowledge refs are `/files/knowledge/*.ref`.
pub const KNOWLEDGE_ROOT: &str = "/files/knowledge";

/// Suffix marking a reference file under [`KNOWLEDGE_ROOT`].
pub const REF_SUFFIX: &str = ".ref";

/// Resident-byte budget per ingest batch: bytes are dropped between batches so
/// peak memory stays bounded. A file over budget forms its own batch (bounded
/// instead by [`MAX_FILE_BYTES`]). Near `Store`'s ~128 MB `IndexWriter`.
const BATCH_BUDGET_BYTES: usize = 128 * 1024 * 1024;

/// Per-file cap. A file can't be streamed (read whole to hash/translate), so an
/// arbitrarily large one could OOM; files over this are skipped.
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

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

/// All per-workspace index state in one struct, so `Resyncer` holds a single
/// `DashMap<Uuid, Arc<WsIndex>>` — one lookup per workspace and a single-entry
/// teardown. `store` opens lazily; `lock` serializes reconciles (and teardown);
/// `refmaps` is the per-target memo; `rev`/`done` are the coalescing gate (a
/// trigger bumps `rev`, a task reconciles only while `rev != done`, and `done`
/// advances — to the pre-scan `rev` — on success).
#[derive(Default)]
struct WsIndex {
    store: OnceCell<SharedStore>,
    lock: Mutex<()>,
    refmaps: StdMutex<HashMap<String, TargetState>>,
    rev: AtomicU64,
    done: AtomicU64,
}

impl WsIndex {
    /// The [`Store`] at `root`, opened (and cached) on first use.
    async fn store(&self, root: PathBuf) -> anyhow::Result<SharedStore> {
        self.store
            .get_or_try_init(|| async { anyhow::Ok(Arc::new(RwLock::new(Store::new(root)?))) })
            .await
            .map(|s| s.clone())
    }
}

/// Indexes a workspace's referenced knowledge into its [`Store`]. Cheap to clone,
/// so one handle serves the fs write-trigger hook, the router, and the loop.
#[derive(Clone)]
pub struct Resyncer {
    db: SqlitePool,
    data_root: PathBuf,
    /// One [`WsIndex`] per workspace (store + lock + memo + coalescing gate).
    indexes: Arc<DashMap<Uuid, Arc<WsIndex>>>,
}

impl Resyncer {
    pub fn new(db: SqlitePool, data_root: PathBuf) -> Self {
        Self {
            db,
            data_root,
            indexes: Arc::new(DashMap::new()),
        }
    }

    fn ws_for(&self, wid: Uuid) -> Arc<WsIndex> {
        self.indexes.entry(wid).or_default().clone()
    }

    fn store_root(&self, wid: Uuid) -> PathBuf {
        self.data_root
            .join("workspaces")
            .join(wid.to_string())
            .join("knowledge-index")
    }

    /// Drop a torn-down workspace's index state (store, memo, gate), releasing its
    /// mmapped tantivy index. Takes the per-workspace lock first so it can't
    /// interleave with an in-flight reconcile.
    pub async fn forget(&self, wid: Uuid) {
        let ws = self.ws_for(wid);
        let _guard = ws.lock.lock().await;
        self.indexes.remove(&wid);
    }

    /// Request a resync for `wid`. Bumps the revision gate and spawns a task; the
    /// [`coalesce`] gate ensures a mutation storm collapses to ~one reconcile
    /// (plus one if a trigger lands mid-run) rather than one per trigger.
    pub fn spawn_resync(&self, wid: Uuid) {
        let ws = self.ws_for(wid);
        ws.rev.fetch_add(1, Ordering::SeqCst);
        let this = self.clone();
        tokio::spawn(async move { coalesce(wid, &ws, || this.reconcile(wid, &ws)).await });
    }

    /// Reconcile `wid`'s [`Store`] with its `/files/knowledge` references: ingest
    /// referenced targets in byte-bounded batches, then purge documents whose ref
    /// is gone. Not self-serializing — callers run it under `ws_index.lock` via
    /// [`coalesce`] (or [`forget`](Self::forget)).
    async fn reconcile(&self, wid: Uuid, ws_index: &WsIndex) -> anyhow::Result<()> {
        // A hookless read view of the unified tree (local `files/` + providers):
        // no resync hook, so reads made here can't re-trigger a reconcile.
        // `from_spec` builds the provider clients synchronously; the async
        // `Mountable` ops are awaited directly on this runtime.
        let mounts = build_workspace_vfs(&self.db, wid).await?;
        let spec = cortex_workspace_spec(&self.data_root, wid, mounts);
        let ws = Arc::new(
            Workspace::from_spec(&spec).map_err(|e| anyhow::anyhow!("cortex workspace: {e:?}"))?,
        );
        let targets = collect_targets(&ws, MAX_FILE_BYTES).await?;
        let member_count = targets.len();
        if member_count == 0 && !self.store_root(wid).join("index").exists() {
            return Ok(());
        }

        let store = ws_index.store(self.store_root(wid)).await?;
        let mut store = store.write().await;

        let prev = ws_index.refmaps.lock().unwrap().clone();

        // Read each target (remembering its content-hash id, or that it failed)
        // and ingest readable bytes in byte-bounded batches.
        let mut succeeded: HashSet<Uuid> = HashSet::new();
        let mut reads: Vec<(String, Option<Uuid>)> = Vec::new();
        let mut batch: Vec<(Vec<u8>, FileType)> = Vec::new();
        let mut batch_bytes = 0usize;
        for (path, ft) in targets {
            let read = read_all(&ws, &path).await;
            match read {
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
        *ws_index.refmaps.lock().unwrap() = next;

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

/// Coalescing gate: reconcile only if a trigger arrived since the last successful
/// pass (`rev != done`), advancing `done` to the pre-scan `rev` on success so a
/// burst's remaining tasks collapse. A failed pass leaves `done` behind, so the
/// burst's other tasks retry (self-heal). Serialized per workspace by `lock`.
async fn coalesce<F, Fut>(wid: Uuid, ws: &WsIndex, reconcile: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    if ws.rev.load(Ordering::SeqCst) == ws.done.load(Ordering::SeqCst) {
        return; // fast path — latest already reconciled; skip the lock
    }
    let _guard = ws.lock.lock().await;
    let cur = ws.rev.load(Ordering::SeqCst);
    if cur == ws.done.load(Ordering::SeqCst) {
        return; // an earlier task already reconciled this revision
    }
    match reconcile().await {
        Ok(()) => ws.done.store(cur, Ordering::SeqCst),
        Err(e) => tracing::warn!(%wid, "knowledge resync failed: {e:#}"),
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
async fn read_all(ws: &Workspace, path: &str) -> anyhow::Result<Vec<u8>> {
    let (handle, stat) = ws
        .open(
            &cx(path),
            OpenOptions {
                read: true,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("open {path}: {e:?}"))?;
    let mut out = vec![0u8; stat.size as usize];
    if stat.size > 0 {
        handle
            .read_exact_at(&mut out, 0)
            .await
            .map_err(|e| anyhow::anyhow!("read {path}: {e}"))?;
    }
    Ok(out)
}

/// Walk `/files/knowledge`, resolve each `*.ref` to its target(s), and return the
/// indexable files' paths + types — without reading their bytes.
async fn collect_targets(
    ws: &Workspace,
    max_file_bytes: u64,
) -> anyhow::Result<Vec<(String, FileType)>> {
    let mut out = Vec::new();
    let entries = match ws.list(&cx(KNOWLEDGE_ROOT)).await {
        Ok(e) => e,
        Err(CortexError::NotFound) => return Ok(out),
        Err(e) => anyhow::bail!("read {KNOWLEDGE_ROOT}: {e:?}"),
    };
    for entry in entries {
        let name = entry.name;
        if !name.ends_with(REF_SUFFIX) {
            continue;
        }
        let ref_path = format!("{KNOWLEDGE_ROOT}/{name}");
        let bytes = match read_all(ws, &ref_path).await {
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
        add_target(ws, &target, max_file_bytes, &mut out).await?;
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
    ws: &Workspace,
    target: &str,
    max_file_bytes: u64,
    out: &mut Vec<(String, FileType)>,
) -> anyhow::Result<()> {
    let meta = match ws.stat(&cx(target)).await {
        Ok(m) => m,
        Err(CortexError::NotFound | CortexError::PermissionDenied) => {
            tracing::warn!("skipping unresolvable target {target}");
            return Ok(());
        }
        Err(e) => anyhow::bail!("stat {target}: {e:?}"),
    };

    if meta.kind != DirentKind::Dir {
        if let Some(ft) = FileType::from_path(Path::new(target)) {
            if meta.size > max_file_bytes {
                tracing::warn!("skipping oversized target {target} ({} bytes)", meta.size);
            } else {
                out.push((target.to_string(), ft));
            }
        }
        return Ok(());
    }

    let mut stack = vec![target.trim_end_matches('/').to_string()];
    while let Some(dir) = stack.pop() {
        let entries = ws
            .list(&cx(&dir))
            .await
            .map_err(|e| anyhow::anyhow!("read {dir}: {e:?}"))?;
        for entry in entries {
            let child = format!("{dir}/{}", entry.name);
            if entry.kind == DirentKind::Dir {
                stack.push(child);
            } else if let Some(ft) = FileType::from_path(Path::new(&entry.name)) {
                // Prefer the in-list stat (an object store returns it for free);
                // fall back to a stat call (a local passthrough may omit it).
                let len = match entry.stat {
                    Some(s) => s.size,
                    None => ws.stat(&cx(&child)).await.map(|s| s.size).unwrap_or(0),
                };
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

/// True when `path` is `/files/knowledge` or lives under it. Shared with the
/// backend's `/knowledge` change hook so the predicate lives in one place.
pub(super) fn is_under_knowledge(path: &str) -> bool {
    let p = path.trim_start_matches('/');
    p == "files/knowledge" || p.starts_with("files/knowledge/")
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cortex::{VolumeSpec, Workspace, WorkspaceSpec};
    use uuid::Uuid;

    use super::{
        KnowledgeRef, Outcome, Resyncer, TargetState, WsIndex, build_keepset, coalesce,
        collect_targets, is_safe_target, read_all,
    };

    fn indexed(id: Uuid) -> TargetState {
        TargetState {
            last_indexed_id: Some(id),
            outcome: Outcome::Indexed,
        }
    }

    // --- build_keepset (pure) --------------------------------------------

    /// A still-referenced target that fails to read keeps its document (protected
    /// via its prior id) instead of being purged on a transient blip.
    #[test]
    fn transient_read_failure_protects_prior_doc() {
        let id = Uuid::from_u128(1);
        let mut prev = HashMap::new();
        prev.insert("/files/a.md".to_string(), indexed(id));
        let reads = vec![("/files/a.md".to_string(), None)];
        let (next, protected) = build_keepset(&prev, &reads, &HashSet::new());
        assert!(protected.contains(&id));
        assert_eq!(next["/files/a.md"].last_indexed_id, Some(id));
        assert_eq!(next["/files/a.md"].outcome, Outcome::ReadFailed);
    }

    /// A removed ref is absent from the scan, so its id isn't protected → purged.
    #[test]
    fn removed_ref_is_purged() {
        let id = Uuid::from_u128(1);
        let mut prev = HashMap::new();
        prev.insert("/files/a.md".to_string(), indexed(id));
        let (next, protected) = build_keepset(&prev, &[], &HashSet::new());
        assert!(!protected.contains(&id));
        assert!(next.is_empty());
    }

    /// A never-indexed target that fails to read has no doc to protect.
    #[test]
    fn unreadable_never_indexed_protects_nothing() {
        let reads = vec![("/files/a.md".to_string(), None)];
        let (next, protected) = build_keepset(&HashMap::new(), &reads, &HashSet::new());
        assert!(protected.is_empty());
        assert_eq!(next["/files/a.md"].last_indexed_id, None);
    }

    /// A broken (unreadable, never-indexed) ref can't shield unrelated docs.
    #[test]
    fn broken_ref_does_not_block_unrelated_purge() {
        let removed_doc = Uuid::from_u128(9);
        let reads = vec![("/mnt/broken.pdf".to_string(), None)];
        let (_, protected) = build_keepset(&HashMap::new(), &reads, &HashSet::new());
        assert!(protected.is_empty());
        assert!(!protected.contains(&removed_doc));
    }

    /// Read OK but ingest failed (new id not confirmed): keep the prior doc.
    #[test]
    fn ingest_failure_keeps_prior_doc() {
        let old = Uuid::from_u128(1);
        let new = Uuid::from_u128(2);
        let mut prev = HashMap::new();
        prev.insert("/files/a.md".to_string(), indexed(old));
        let reads = vec![("/files/a.md".to_string(), Some(new))];
        let (next, protected) = build_keepset(&prev, &reads, &HashSet::new());
        assert!(protected.contains(&old));
        assert_eq!(next["/files/a.md"].last_indexed_id, Some(old));
        assert_eq!(next["/files/a.md"].outcome, Outcome::IngestFailed);
    }

    /// The memo carries a target's last indexed id across cycles: it survives
    /// repeated read blips and re-indexes on recovery (output fed back as `prev`).
    #[test]
    fn refmaps_carries_id_across_cycles() {
        let p = "/files/a.md".to_string();
        let x = Uuid::from_u128(1);
        let hit: HashSet<Uuid> = [x].into_iter().collect();

        let (m1, prot) = build_keepset(&HashMap::new(), &[(p.clone(), Some(x))], &hit);
        assert!(prot.contains(&x));
        assert_eq!(m1[&p].last_indexed_id, Some(x));

        let (m2, prot) = build_keepset(&m1, &[(p.clone(), None)], &HashSet::new());
        assert!(prot.contains(&x));
        let (m3, prot) = build_keepset(&m2, &[(p.clone(), None)], &HashSet::new());
        assert!(prot.contains(&x));

        let (m4, prot) = build_keepset(&m3, &[(p.clone(), Some(x))], &hit);
        assert!(prot.contains(&x));
        assert_eq!(m4[&p].outcome, Outcome::Indexed);
    }

    /// Read OK, content changed, ingest confirmed: protect the new id; the old
    /// content's doc is unreferenced → purgeable.
    #[test]
    fn content_change_swaps_protected_id() {
        let old = Uuid::from_u128(1);
        let new = Uuid::from_u128(2);
        let mut prev = HashMap::new();
        prev.insert("/files/a.md".to_string(), indexed(old));
        let reads = vec![("/files/a.md".to_string(), Some(new))];
        let succeeded: HashSet<Uuid> = [new].into_iter().collect();
        let (next, protected) = build_keepset(&prev, &reads, &succeeded);
        assert!(protected.contains(&new));
        assert!(!protected.contains(&old));
        assert_eq!(next["/files/a.md"].last_indexed_id, Some(new));
        assert_eq!(next["/files/a.md"].outcome, Outcome::Indexed);
    }

    /// One keep-set over a mixed batch: each target's fate is independent.
    #[test]
    fn mixed_batch_protects_each_target_independently() {
        let ok_id = Uuid::from_u128(1);
        let blip_prior = Uuid::from_u128(2);
        let removed_id = Uuid::from_u128(3);
        let unconfirmed = Uuid::from_u128(4);

        let mut prev = HashMap::new();
        prev.insert("/files/ok.md".to_string(), indexed(ok_id));
        prev.insert("/files/blip.md".to_string(), indexed(blip_prior));
        prev.insert("/files/removed.md".to_string(), indexed(removed_id));

        let reads = vec![
            ("/files/ok.md".to_string(), Some(ok_id)),
            ("/files/blip.md".to_string(), None),
            ("/files/new.md".to_string(), Some(unconfirmed)),
        ];
        let succeeded: HashSet<Uuid> = [ok_id].into_iter().collect();

        let (next, protected) = build_keepset(&prev, &reads, &succeeded);

        assert_eq!(protected, [ok_id, blip_prior].into_iter().collect());
        assert!(!protected.contains(&removed_id));
        assert!(!protected.contains(&unconfirmed));
        assert_eq!(next["/files/ok.md"].outcome, Outcome::Indexed);
        assert_eq!(next["/files/blip.md"].outcome, Outcome::ReadFailed);
        assert_eq!(next["/files/new.md"].outcome, Outcome::IngestFailed);
        assert_eq!(next["/files/new.md"].last_indexed_id, None);
        assert!(!next.contains_key("/files/removed.md"));
    }

    // --- enumeration -----------------------------------------------------

    fn write(root: &Path, rel: &str, body: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn write_ref(root: &Path, name: &str, target: &str) {
        let body = KnowledgeRef {
            path: target.into(),
        }
        .to_bytes();
        write(root, &format!("knowledge/{name}"), &body);
    }

    /// A local `files` mount over `root` plus a second local `mock` mount —
    /// enough to exercise cross-mount enumeration.
    fn ws_with_mock(root: &Path, mock_root: &Path) -> Workspace {
        let spec = WorkspaceSpec::default()
            .mount("files", VolumeSpec::Local { host: root.to_path_buf() })
            .mount("mock", VolumeSpec::Local { host: mock_root.to_path_buf() });
        Workspace::from_spec(&spec).unwrap()
    }

    /// A local-only `files` mount over `root`.
    fn local_ws(root: &Path) -> Workspace {
        let spec = WorkspaceSpec::default().mount("files", VolumeSpec::Local {
            host: root.to_path_buf(),
        });
        Workspace::from_spec(&spec).unwrap()
    }

    /// Enumeration gathers local files, a directory's descendants, and a file on
    /// a second mount — skipping non-`.ref`, self-references, missing targets,
    /// and non-indexable extensions — and the paths resolve to real bytes.
    #[tokio::test]
    async fn collects_local_dir_and_mount_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let mock = tempfile::tempdir().unwrap();
        let mock_root = mock.path();

        write(root, "docs/a.md", b"AAA");
        write(root, "docs/sub/b.md", b"BBB");
        write(root, "docs/sub/ignore.bin", b"nope");
        write(mock_root, "doc.md", b"MOUNTED");

        write_ref(root, "a.ref", "/files/docs/a.md");
        write_ref(root, "dir.ref", "/files/docs/sub");
        write_ref(root, "mount.ref", "/mock/doc.md");
        write_ref(root, "missing.ref", "/files/docs/gone.md");
        write_ref(root, "self.ref", "/files/knowledge/a.ref");
        write(root, "knowledge/note.txt", b"not a ref");

        let ws = ws_with_mock(root, mock_root);

        let targets = collect_targets(&ws, u64::MAX).await.unwrap();
        let mut paths: Vec<String> = targets.iter().map(|(p, _)| p.clone()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "/files/docs/a.md".to_string(),
                "/files/docs/sub/b.md".to_string(),
                "/mock/doc.md".to_string(),
            ]
        );

        let mut bodies = Vec::new();
        for (p, _) in &targets {
            bodies.push(read_all(&ws, p).await.unwrap());
        }
        bodies.sort();
        assert_eq!(
            bodies,
            vec![b"AAA".to_vec(), b"BBB".to_vec(), b"MOUNTED".to_vec()]
        );
    }

    /// A directory ref and an explicit file ref covering the same path enumerate
    /// it once.
    #[tokio::test]
    async fn dedups_overlapping_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "docs/a.md", b"A");
        write_ref(root, "dir.ref", "/files/docs");
        write_ref(root, "file.ref", "/files/docs/a.md");
        let ws = local_ws(root);

        let targets = collect_targets(&ws, u64::MAX).await.unwrap();
        let paths: Vec<String> = targets.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(paths, vec!["/files/docs/a.md".to_string()]);
    }

    /// A target larger than the per-file cap is skipped, smaller ones kept.
    #[tokio::test]
    async fn oversized_target_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "docs/small.md", b"tiny");
        write(root, "docs/big.md", &vec![b'x'; 100]);
        write_ref(root, "small.ref", "/files/docs/small.md");
        write_ref(root, "big.ref", "/files/docs/big.md");
        let ws = local_ws(root);

        let targets = collect_targets(&ws, 10).await.unwrap();
        let paths: Vec<String> = targets.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(paths, vec!["/files/docs/small.md".to_string()]);
    }

    /// A planted reference whose target escapes the root (`..`) or is relative is
    /// skipped, not resolved.
    #[tokio::test]
    async fn skips_unsafe_ref_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_ref(root, "esc.ref", "/../outside.md");
        write_ref(root, "rel.ref", "docs/x.md");
        write(root, "docs/ok.md", b"OK");
        write_ref(root, "ok.ref", "/files/docs/ok.md");
        let ws = local_ws(root);

        let targets = collect_targets(&ws, u64::MAX).await.unwrap();
        let paths: Vec<String> = targets.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(paths, vec!["/files/docs/ok.md".to_string()]);
    }

    /// A missing `/files/knowledge` directory yields no targets rather than erroring.
    #[tokio::test]
    async fn absent_knowledge_dir_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = local_ws(tmp.path());
        let targets = collect_targets(&ws, u64::MAX).await.unwrap();
        assert!(targets.is_empty());
    }

    #[test]
    fn safe_target_rules() {
        assert!(is_safe_target("/files/a.md"));
        assert!(!is_safe_target("a.md"));
        assert!(!is_safe_target("/a/../b"));
        assert!(!is_safe_target("/"));
    }

    // --- coalescing gate -------------------------------------------------

    /// A burst of triggers runs one reconcile; a later trigger runs one more.
    #[tokio::test]
    async fn coalesces_burst_to_one_reconcile() {
        let ws = WsIndex::default();
        let wid = Uuid::new_v4();
        let calls = AtomicUsize::new(0);
        for _ in 0..5 {
            ws.rev.fetch_add(1, Ordering::SeqCst);
        }
        for _ in 0..5 {
            coalesce(wid, &ws, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                anyhow::Ok(())
            })
            .await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        ws.rev.fetch_add(1, Ordering::SeqCst);
        coalesce(wid, &ws, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            anyhow::Ok(())
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// On Err, `done` stays behind `rev`, so the burst's remaining tasks retry.
    #[tokio::test]
    async fn err_does_not_advance_done_so_burst_retries() {
        let ws = WsIndex::default();
        let wid = Uuid::new_v4();
        ws.rev.fetch_add(1, Ordering::SeqCst);
        coalesce(wid, &ws, || async { anyhow::bail!("transient") }).await;

        let calls = AtomicUsize::new(0);
        coalesce(wid, &ws, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            anyhow::Ok(())
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// A trigger during a reconcile leaves `done` behind `rev`, so exactly one
    /// follow-up runs (guards "never re-read `rev` at the end").
    #[tokio::test]
    async fn trigger_during_reconcile_runs_one_more() {
        let ws = WsIndex::default();
        let wid = Uuid::new_v4();
        let calls = AtomicUsize::new(0);

        ws.rev.fetch_add(1, Ordering::SeqCst);
        coalesce(wid, &ws, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            ws.rev.fetch_add(1, Ordering::SeqCst);
            anyhow::Ok(())
        })
        .await;
        assert_eq!(ws.done.load(Ordering::SeqCst), 1);

        for _ in 0..3 {
            coalesce(wid, &ws, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                anyhow::Ok(())
            })
            .await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// Concurrent triggers coalesce to exactly one reconcile under real contention.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_triggers_coalesce_to_one() {
        use tokio::sync::oneshot;

        let ws = Arc::new(WsIndex::default());
        let wid = Uuid::new_v4();
        let calls = Arc::new(AtomicUsize::new(0));
        ws.rev.fetch_add(1, Ordering::SeqCst);
        ws.rev.fetch_add(1, Ordering::SeqCst);

        let (in_tx, in_rx) = oneshot::channel();
        let (rel_tx, rel_rx) = oneshot::channel();

        let a = tokio::spawn({
            let (ws, calls) = (ws.clone(), calls.clone());
            async move {
                coalesce(wid, &ws, move || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    in_tx.send(()).unwrap();
                    rel_rx.await.unwrap();
                    anyhow::Ok(())
                })
                .await;
            }
        });
        in_rx.await.unwrap();

        let b = tokio::spawn({
            let (ws, calls) = (ws.clone(), calls.clone());
            async move {
                coalesce(wid, &ws, || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    anyhow::Ok(())
                })
                .await;
            }
        });

        rel_tx.send(()).unwrap();
        a.await.unwrap();
        b.await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(ws.done.load(Ordering::SeqCst), 2);
    }

    /// `forget` drops a deleted workspace's index state.
    #[tokio::test]
    async fn forget_evicts_workspace_state() {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let r = Resyncer::new(pool, tmp.path().to_path_buf());
        let wid = Uuid::new_v4();

        r.ws_for(wid);
        assert!(r.indexes.contains_key(&wid));

        r.forget(wid).await;
        assert!(!r.indexes.contains_key(&wid));
    }
}
