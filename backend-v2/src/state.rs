use std::collections::HashMap;

use ailoy::agent::Agent;
use uuid::Uuid;

pub struct AppState {
    agents: HashMap<Uuid, Agent>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    pub fn insert_agent(&mut self, id: Uuid, agent: Agent) {
        self.agents.insert(id, agent);
    }
}
