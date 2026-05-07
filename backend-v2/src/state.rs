use std::sync::Arc;

use ailoy::agent::Agent;
use dashmap::DashMap;
use speedwagon::SharedStore;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{auth::JwtConfig, repository::AppRepository};

pub struct AppState {
    agents: DashMap<Uuid, Arc<Mutex<Agent>>>,
    pub repository: AppRepository,
    pub store: SharedStore,
    pub jwt: JwtConfig,
}

impl AppState {
    pub fn new(repository: AppRepository, store: SharedStore, jwt: JwtConfig) -> Self {
        Self {
            agents: DashMap::new(),
            repository,
            store,
            jwt,
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
