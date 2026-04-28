use std::sync::Arc;

use ailoy::agent::Agent;
use dashmap::DashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct AppState {
    agents: DashMap<Uuid, Arc<Mutex<Agent>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
        }
    }

    pub fn insert_agent(&self, id: Uuid, agent: Agent) {
        self.agents.insert(id, Arc::new(Mutex::new(agent)));
    }

    pub fn get_agent(&self, id: &Uuid) -> Option<Arc<Mutex<Agent>>> {
        self.agents.get(id).map(|entry| entry.value().clone())
    }
}
