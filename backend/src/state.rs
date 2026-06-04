use std::{path::PathBuf, sync::Arc};

use agent_k::knowledge_base::SharedStore;
use ailoy::{agent::Agent, message::MessageOutput};
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    auth::JwtConfig,
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
    pub store: SharedStore,
    pub jwt: JwtConfig,
    pub data_root: PathBuf,
    pub max_upload_bytes: usize,
    pub ws_tx: broadcast::Sender<WsEvent>,
}

impl AppState {
    pub fn new(
        repository: AppRepository,
        store: SharedStore,
        jwt: JwtConfig,
        data_root: PathBuf,
    ) -> Self {
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
            store,
            jwt,
            data_root,
            max_upload_bytes,
            ws_tx,
        }
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
