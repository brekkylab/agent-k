use std::collections::HashMap;
use std::sync::Arc;

use ailoy::agent::Agent;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::repository::AppRepository;

pub struct AppState {
    agents: HashMap<Uuid, Arc<Mutex<Agent>>>,
    pub repository: AppRepository,
}

impl AppState {
    pub fn new(repository: AppRepository) -> Self {
        Self {
            agents: HashMap::new(),
            repository,
        }
    }

    pub fn insert_agent(&mut self, id: Uuid, agent: Agent) {
        self.agents.insert(id, Arc::new(Mutex::new(agent)));
    }

    pub fn remove_agent(&mut self, id: &Uuid) -> Option<Arc<Mutex<Agent>>> {
        self.agents.remove(id)
    }

    pub fn get_agent(&self, id: &Uuid) -> Option<Arc<Mutex<Agent>>> {
        self.agents.get(id).cloned()
    }
}
