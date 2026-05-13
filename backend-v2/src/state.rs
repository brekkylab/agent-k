use std::{path::PathBuf, sync::Arc};

use agent_k::knowledge_base::SharedStore;
use ailoy::agent::Agent;
use dashmap::DashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{auth::JwtConfig, repository::AppRepository};

pub struct AppState {
    agents: DashMap<Uuid, Arc<Mutex<Agent>>>,
    pub repository: AppRepository,
    pub store: SharedStore,
    pub jwt: JwtConfig,
    pub data_root: PathBuf,
    pub max_upload_bytes: usize,
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
        Self {
            agents: DashMap::new(),
            repository,
            store,
            jwt,
            data_root,
            max_upload_bytes,
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
}
