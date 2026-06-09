use std::{path::PathBuf, sync::Arc};

use agent_k::knowledge_base::{SharedStore, Store};
use ailoy::{agent::Agent, message::MessageOutput};
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock, broadcast};
use uuid::Uuid;

use crate::{
    auth::JwtConfig,
    error::{ApiError, AppError},
    events::{RunUserMessage, WsEvent},
    repository::AppRepository,
};

pub struct ActiveAgentRun {
    pub run_id: Uuid,
    pub user_message: RunUserMessage,
    pub(crate) next_seq: u64,
    pub(crate) outputs: Vec<(u64, MessageOutput)>,
}

pub struct AppState {
    agents: DashMap<Uuid, Arc<Mutex<Agent>>>,
    active_agent_runs: DashMap<Uuid, Arc<RwLock<ActiveAgentRun>>>,
    pub repository: AppRepository,
    /// Per-project document corpora (Speedwagon). Each project gets its own
    /// on-disk store under `data_root/projects/{project_id}/.speedwagon`,
    /// opened lazily on first access via [`AppState::store_for`].
    document_stores: DashMap<Uuid, SharedStore>,
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
            jwt,
            data_root,
            max_upload_bytes,
            ws_tx,
        }
    }

    /// Return the document corpus [`SharedStore`] for `project_id`, opening it
    /// on first access. The store lives at
    /// `data_root/projects/{project_id}/.speedwagon`; its directories are
    /// created if absent.
    pub async fn store_for(&self, project_id: Uuid) -> Result<SharedStore, ApiError> {
        if let Some(store) = self.document_stores.get(&project_id) {
            return Ok(store.clone());
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
        // Race-safe: if another task opened it first between the get() above
        // and here, keep the existing entry.
        Ok(self
            .document_stores
            .entry(project_id)
            .or_insert(shared)
            .clone())
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

    /// Registers an active run for `session_id` and returns the generated `run_id`.
    ///
    /// Caller holds the agent `OwnedMutexGuard`, proving no real active run exists.
    /// If a stale entry is found it is force-replaced and a warning is logged.
    pub fn start_run(&self, session_id: Uuid, user_message: RunUserMessage) -> Uuid {
        let run_id = Uuid::new_v4();
        let fresh = Arc::new(RwLock::new(ActiveAgentRun {
            run_id,
            user_message,
            next_seq: 0,
            outputs: vec![],
        }));
        if self.active_agent_runs.insert(session_id, fresh).is_some() {
            // Caller holds the agent lock, so no real run is executing.
            // A remaining entry is a leaked end_run — replace it and warn.
            tracing::warn!(%session_id, "start_run: replaced stale active-run entry (previous run leaked end_run)");
        }
        run_id
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
