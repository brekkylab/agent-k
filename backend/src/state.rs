use std::{
    path::PathBuf,
    sync::Arc,
    time::{Instant, SystemTime},
};

use agent_k::knowledge_base::{SharedStore, Store};
use ailoy::{agent::Agent, message::MessageOutput};
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Cap on simultaneously open per-project corpus stores. Each holds a tantivy
/// index (open file handles); LRU eviction keeps the count bounded.
const MAX_OPEN_STORES: usize = 64;

/// A cached corpus store plus the last time it was handed out, for LRU eviction.
struct StoreEntry {
    store: SharedStore,
    last_access: Instant,
}

use crate::{
    auth::JwtConfig,
    error::{ApiError, AppError},
    events::{RunUserMessage, WsEvent},
    repository::AppRepository,
};

pub struct ActiveAgentRun {
    pub run_id: Uuid,
    pub user_message: RunUserMessage,
    pub cancel: CancellationToken,
    pub(crate) next_seq: u64,
    pub(crate) outputs: Vec<(u64, MessageOutput)>,
}

pub struct AppState {
    agents: DashMap<Uuid, Arc<Mutex<Agent>>>,
    active_agent_runs: DashMap<Uuid, Arc<RwLock<ActiveAgentRun>>>,
    pub repository: AppRepository,
    /// Per-project document corpora (Speedwagon), opened lazily and capped at
    /// [`MAX_OPEN_STORES`] with LRU eviction so a long-running server doesn't
    /// hold an unbounded number of open tantivy indices.
    document_stores: DashMap<Uuid, StoreEntry>,
    /// In-flight knowledge resync count per project (>0 means indexing).
    /// `Arc` so an [`IndexingGuard`] can decrement it on drop, surviving early
    /// returns and panics in the background resync task.
    knowledge_indexing: Arc<DashMap<Uuid, u32>>,
    /// Cache of each knowledge file's content id, keyed by (project, path) and
    /// validated against (mtime, size). Lets the per-file status endpoint skip
    /// re-reading and re-hashing files that haven't changed between polls.
    knowledge_file_ids: DashMap<(Uuid, PathBuf), (SystemTime, u64, Uuid)>,
    /// Last knowledge-resync error per project, surfaced in the status endpoint.
    /// Set when a background resync fails, cleared when one succeeds.
    knowledge_resync_error: DashMap<Uuid, String>,
    /// Scope-relative paths that failed to index in the latest resync, replaced
    /// each resync. Lets the status endpoint mark a file failed, not "pending".
    knowledge_failed_files: DashMap<Uuid, Arc<std::collections::HashSet<String>>>,
    // These two lock maps are intentionally never pruned (e.g. on evict_store):
    // a task can hold a project's lock while another evicts that project, so
    // removing the entry would let a fresh `entry().or_insert()` mint a second
    // Mutex and break serialization. They are bounded by project count (one
    // small Mutex each), so the unbounded-key concern does not apply.
    /// Per-project lock serializing resyncs so two rescans never interleave.
    resync_locks: DashMap<Uuid, Arc<Mutex<()>>>,
    /// Per-project lock serializing first-time store creation. `Store::new`
    /// (tantivy `open_or_create`) is not safe to run twice concurrently on the
    /// same directory — two racing opens hit a create/remove TOCTOU and one
    /// fails with "No such file or directory". Holding this across the
    /// open-and-cache step means only one task ever opens a given store.
    store_init_locks: DashMap<Uuid, Arc<Mutex<()>>>,
    /// Per-project (title, line_count) for citation checks; refreshed by resync,
    /// read on message fetch so it never loads corpus content on that path.
    corpus_summaries: DashMap<Uuid, Arc<Vec<(String, usize)>>>,
    /// Cached citation-verification results, keyed by `(project, session, seq)`.
    /// Lets the message-fetch path reuse a check instead of recomputing it each
    /// poll; dropped per project when the corpus changes so it never goes stale.
    citation_checks: DashMap<(Uuid, Uuid, i64), Arc<Vec<crate::handlers::knowledge::CitationCheck>>>,
    pub jwt: JwtConfig,
    pub data_root: PathBuf,
    pub max_upload_bytes: usize,
    pub ws_tx: broadcast::Sender<WsEvent>,
}

impl AppState {
    pub fn new(repository: AppRepository, jwt: JwtConfig, data_root: PathBuf) -> Self {
        let max_upload_bytes = std::env::var("AGENT_K_MAX_UPLOAD_BYTES")
            .ok()
            .and_then(|v| {
                v.parse()
                    .map_err(|_| {
                        tracing::warn!(
                            "invalid AGENT_K_MAX_UPLOAD_BYTES value '{v}', using default"
                        )
                    })
                    .ok()
            })
            .unwrap_or(50 * 1024 * 1024);
        let (ws_tx, _) = broadcast::channel(128);
        Self {
            agents: DashMap::new(),
            active_agent_runs: DashMap::new(),
            repository,
            document_stores: DashMap::new(),
            knowledge_file_ids: DashMap::new(),
            knowledge_resync_error: DashMap::new(),
            knowledge_failed_files: DashMap::new(),
            knowledge_indexing: Arc::new(DashMap::new()),
            resync_locks: DashMap::new(),
            store_init_locks: DashMap::new(),
            corpus_summaries: DashMap::new(),
            citation_checks: DashMap::new(),
            jwt,
            data_root,
            max_upload_bytes,
            ws_tx,
        }
    }

    /// Mark a knowledge resync as in flight for `project_id`, returning a guard
    /// that decrements the count when dropped. Using a guard (rather than a
    /// paired `end_indexing` call) keeps the count correct even if the resync
    /// returns early or panics, so the UI never gets stuck showing "indexing".
    pub fn begin_indexing(&self, project_id: Uuid) -> IndexingGuard {
        *self.knowledge_indexing.entry(project_id).or_insert(0) += 1;
        IndexingGuard {
            counts: self.knowledge_indexing.clone(),
            project_id,
        }
    }

    /// Whether a knowledge resync is currently in flight for `project_id`.
    pub fn is_indexing(&self, project_id: Uuid) -> bool {
        self.knowledge_indexing.get(&project_id).map(|n| *n > 0).unwrap_or(false)
    }

    /// Per-project resync lock; the caller holds it across the whole resync.
    pub fn resync_lock_for(&self, project_id: Uuid) -> Arc<Mutex<()>> {
        self.resync_locks
            .entry(project_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .value()
            .clone()
    }

    /// Record the outcome of a resync: `Some(msg)` on failure, `None` clears a
    /// prior error after a success. The latest value is surfaced in the status.
    pub fn set_resync_error(&self, project_id: Uuid, error: Option<String>) {
        match error {
            Some(e) => {
                self.knowledge_resync_error.insert(project_id, e);
            }
            None => {
                self.knowledge_resync_error.remove(&project_id);
            }
        }
    }

    /// The last knowledge-resync error for `project_id`, if the most recent
    /// resync failed and none has succeeded since.
    pub fn resync_error(&self, project_id: Uuid) -> Option<String> {
        self.knowledge_resync_error.get(&project_id).map(|e| e.clone())
    }

    /// Replace the set of scope-relative paths that failed to index.
    pub fn set_failed_files(&self, project_id: Uuid, paths: std::collections::HashSet<String>) {
        if paths.is_empty() {
            self.knowledge_failed_files.remove(&project_id);
        } else {
            self.knowledge_failed_files.insert(project_id, Arc::new(paths));
        }
    }

    /// Whether `rel_path` (scope-relative) failed to index in the latest resync.
    pub fn file_failed(&self, project_id: Uuid, rel_path: &str) -> bool {
        self.knowledge_failed_files
            .get(&project_id)
            .is_some_and(|s| s.contains(rel_path))
    }

    /// Cached content id for a knowledge file, valid only if `(mtime, size)`
    /// still match what was cached — otherwise `None` and the caller must
    /// re-hash. Lets the per-file status poll avoid re-reading unchanged files.
    pub fn cached_file_id(&self, project_id: Uuid, path: &std::path::Path, mtime: SystemTime, size: u64) -> Option<Uuid> {
        self.knowledge_file_ids
            .get(&(project_id, path.to_path_buf()))
            .and_then(|e| {
                let (m, s, id) = *e;
                (m == mtime && s == size).then_some(id)
            })
    }

    /// Record a knowledge file's content id alongside its `(mtime, size)`.
    pub fn cache_file_id(&self, project_id: Uuid, path: &std::path::Path, mtime: SystemTime, size: u64, id: Uuid) {
        self.knowledge_file_ids
            .insert((project_id, path.to_path_buf()), (mtime, size, id));
    }

    /// Drop cached file ids for `project_id` whose path is no longer in `keep`.
    /// Called from a resync (which already walked the folder) so entries for
    /// deleted files don't linger until the store is evicted.
    pub fn retain_file_ids(&self, project_id: Uuid, keep: &std::collections::HashSet<PathBuf>) {
        self.knowledge_file_ids
            .retain(|(pid, path), _| *pid != project_id || keep.contains(path));
    }

    /// Cached `(title, line_count)` summary of a project's corpus, if computed.
    pub fn corpus_summary(&self, project_id: Uuid) -> Option<Arc<Vec<(String, usize)>>> {
        self.corpus_summaries.get(&project_id).map(|e| e.clone())
    }

    /// Replace the cached corpus summary (called after a resync rebuilds it).
    pub fn set_corpus_summary(&self, project_id: Uuid, summary: Vec<(String, usize)>) {
        self.corpus_summaries.insert(project_id, Arc::new(summary));
    }

    /// Cached citation checks for one message, if computed since the corpus last
    /// changed.
    pub fn citation_checks(
        &self,
        project_id: Uuid,
        session_id: Uuid,
        seq: i64,
    ) -> Option<Arc<Vec<crate::handlers::knowledge::CitationCheck>>> {
        self.citation_checks.get(&(project_id, session_id, seq)).map(|e| e.clone())
    }

    /// Cache the citation checks for one message.
    pub fn set_citation_checks(
        &self,
        project_id: Uuid,
        session_id: Uuid,
        seq: i64,
        checks: Vec<crate::handlers::knowledge::CitationCheck>,
    ) {
        self.citation_checks
            .insert((project_id, session_id, seq), Arc::new(checks));
    }

    /// Drop all cached citation checks for a project. Called when its corpus
    /// changes (resync, engine rebuild) so stale checks are recomputed on the
    /// next read against the updated corpus.
    pub fn clear_citation_checks(&self, project_id: Uuid) {
        self.citation_checks.retain(|k, _| k.0 != project_id);
    }

    /// Drop cached citation checks for one session. Called when the session is
    /// torn down so its entries don't linger in the cache forever (corpus-change
    /// eviction is keyed by project, not session).
    pub fn clear_session_citation_checks(&self, session_id: Uuid) {
        self.citation_checks.retain(|k, _| k.1 != session_id);
    }

    /// Return the document corpus [`SharedStore`] for `project_id`, opening it
    /// on first access. The store lives at
    /// `data_root/projects/{project_id}/.speedwagon`; its directories are
    /// created if absent.
    pub async fn store_for(&self, project_id: Uuid) -> Result<SharedStore, ApiError> {
        if let Some(mut e) = self.document_stores.get_mut(&project_id) {
            e.last_access = Instant::now();
            return Ok(e.store.clone());
        }
        // Cache miss: serialize first-time opens per project so `Store::new`
        // never runs twice concurrently on the same directory (tantivy's
        // open_or_create has a create/remove TOCTOU). The task that wins the
        // lock opens and caches; tasks that waited fall through the re-check
        // below and reuse that handle without opening a second one.
        let init_lock = self
            .store_init_locks
            .entry(project_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .value()
            .clone();
        let _init = init_lock.lock().await;
        // Re-check under the lock: a concurrent caller may have opened it while
        // we waited.
        if let Some(mut e) = self.document_stores.get_mut(&project_id) {
            e.last_access = Instant::now();
            return Ok(e.store.clone());
        }
        let root = self
            .data_root
            .join("projects")
            .join(project_id.to_string())
            .join(".speedwagon");
        // `Store::new` does blocking filesystem + tantivy index work.
        let store = tokio::task::spawn_blocking(move || Store::new(&root))
            .await
            .map_err(|e| AppError::internal(format!("store init join error: {e}")))?
            .map_err(|e| AppError::internal(format!("store init failed: {e}")))?;
        let shared: SharedStore = Arc::new(RwLock::new(store));
        // Evict the LRU entry at capacity, skipping projects that are indexing:
        // a resync holds that store's write lock, and reopening it elsewhere
        // would create a second handle fighting tantivy's per-dir writer lock.
        // A non-indexing store in use still survives — it's an `Arc`.
        if self.document_stores.len() >= MAX_OPEN_STORES
            && let Some(oldest) = self
                .document_stores
                .iter()
                .filter(|e| !self.is_indexing(*e.key()))
                .min_by_key(|e| e.last_access)
                .map(|e| *e.key())
        {
            self.evict_store(oldest);
        }
        Ok(self
            .document_stores
            .entry(project_id)
            .or_insert(StoreEntry { store: shared, last_access: Instant::now() })
            .store
            .clone())
    }

    /// Drop the cached corpus store handle for `project_id` and its derived
    /// caches (summary, file-ids). The next [`store_for`](Self::store_for)
    /// reopens from disk and the caches refill, so they stay bounded with the
    /// LRU store cache rather than growing per project forever. Used on the
    /// engine-change rebuild and on LRU eviction.
    pub fn evict_store(&self, project_id: Uuid) {
        self.document_stores.remove(&project_id);
        self.corpus_summaries.remove(&project_id);
        self.knowledge_failed_files.remove(&project_id);
        self.knowledge_file_ids.retain(|k, _| k.0 != project_id);
        self.clear_citation_checks(project_id);
    }

    pub fn insert_agent(&self, id: Uuid, agent: Agent) {
        self.agents.insert(id, Arc::new(Mutex::new(agent)));
    }

    pub fn remove_agent(&self, id: &Uuid) -> Option<Arc<Mutex<Agent>>> {
        self.agents.remove(id).map(|(_, v)| v)
    }

    pub fn get_agent(&self, id: &Uuid) -> Option<Arc<Mutex<Agent>>> {
        self.agents.get(id).map(|entry| entry.value().clone())
    }

    /// Registers an active run for `session_id` and returns the generated `run_id`
    /// plus the run's cancellation token.
    ///
    /// Caller holds the agent `OwnedMutexGuard`, proving no real active run exists.
    /// If a stale entry is found it is force-replaced and a warning is logged.
    pub fn start_run(
        &self,
        session_id: Uuid,
        user_message: RunUserMessage,
    ) -> (Uuid, CancellationToken) {
        let run_id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        let fresh = Arc::new(RwLock::new(ActiveAgentRun {
            run_id,
            user_message,
            cancel: cancel.clone(),
            next_seq: 0,
            outputs: vec![],
        }));
        if self.active_agent_runs.insert(session_id, fresh).is_some() {
            // Caller holds the agent lock, so no real run is executing.
            // A remaining entry is a leaked end_run — replace it and warn.
            tracing::warn!(%session_id, "start_run: replaced stale active-run entry (previous run leaked end_run)");
        }
        (run_id, cancel)
    }

    pub async fn run_cancel_info(
        &self,
        session_id: &Uuid,
    ) -> Option<(Uuid, String, CancellationToken)> {
        let entry = self.active_agent_runs.get(session_id)?;
        let run_arc = entry.value().clone();
        drop(entry);
        let run = run_arc.read().await;
        Some((
            run.run_id,
            run.user_message.sender_user_id.clone(),
            run.cancel.clone(),
        ))
    }

    pub async fn push_output(&self, session_id: &Uuid, output: MessageOutput) -> Option<u64> {
        let entry = self.active_agent_runs.get(session_id)?;
        let run_arc = entry.value().clone();
        drop(entry);
        let mut run = run_arc.write().await;
        let seq = run.next_seq;
        run.next_seq += 1;
        run.outputs.push((seq, output));
        Some(seq)
    }

    pub async fn snapshot(
        &self,
        session_id: &Uuid,
    ) -> Option<(Uuid, RunUserMessage, Vec<(u64, MessageOutput)>)> {
        let entry = self.active_agent_runs.get(session_id)?;
        let run_arc = entry.value().clone();
        drop(entry);
        let run = run_arc.read().await;
        Some((run.run_id, run.user_message.clone(), run.outputs.clone()))
    }

    pub fn has_active_run(&self, session_id: &Uuid) -> bool {
        self.active_agent_runs.contains_key(session_id)
    }

    /// Removes the active run record for `session_id`.
    ///
    /// # Invariant
    /// Must be called exactly once per `start_run`, on both success and error paths.
    /// Failing to call this leaks the in-memory run buffer indefinitely.
    pub fn end_run(&self, session_id: &Uuid) {
        self.active_agent_runs.remove(session_id);
    }
}

/// Decrements a project's knowledge-indexing count when dropped. Held for the
/// duration of a resync so the count is released on every exit path (success,
/// early return, or panic). Obtained from [`AppState::begin_indexing`].
pub struct IndexingGuard {
    counts: Arc<DashMap<Uuid, u32>>,
    project_id: Uuid,
}

impl Drop for IndexingGuard {
    fn drop(&mut self) {
        if let Some(mut n) = self.counts.get_mut(&self.project_id) {
            *n = n.saturating_sub(1);
        }
    }
}
