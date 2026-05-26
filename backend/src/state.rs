use std::{path::PathBuf, sync::Arc};

use agent_k::knowledge_base::{SharedStore, Store};
use ailoy::agent::Agent;
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock, broadcast};
use uuid::Uuid;

use crate::{auth::JwtConfig, events::WsEvent, repository::AppRepository};

pub struct AppState {
    agents: DashMap<Uuid, Arc<Mutex<Agent>>>,
    /// Per-project Speedwagon stores. Lazily created on first `get_store`.
    stores: DashMap<Uuid, SharedStore>,
    pub repository: AppRepository,
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
            stores: DashMap::new(),
            repository,
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

    /// Return the per-project Speedwagon `Store`, creating it on first access.
    /// Index lives under `<data_root>/projects/{id}/.speedwagon`.
    ///
    /// Panics if the index directory cannot be created or opened — at boot
    /// the parent `projects/{id}` directory is already set up by the project
    /// handler, so the only realistic failure is a corrupt on-disk index that
    /// the agent-k Store auto-rebuild cannot recover from.
    pub async fn get_store(&self, project_id: Uuid) -> SharedStore {
        if let Some(s) = self.stores.get(&project_id) {
            return s.value().clone();
        }
        let path = self
            .data_root
            .join("projects")
            .join(project_id.to_string())
            .join(".speedwagon");
        let store = Store::new(&path)
            .unwrap_or_else(|e| panic!("failed to open Speedwagon store at {path:?}: {e}"));
        let shared: SharedStore = Arc::new(RwLock::new(store));
        let entry = self
            .stores
            .entry(project_id)
            .or_insert_with(|| shared.clone());
        entry.value().clone()
    }
}
