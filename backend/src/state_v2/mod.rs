mod agent;
mod db;
mod fs;

// pub use agent::*;
pub use db::*;
pub use fs::*;

pub struct AppStateV2 {
    pub db: DBStateV2,

    pub fs: FSStateV2,
    // pub agents: AgentsStateV2,
}

impl AppStateV2 {
    pub fn new(db: DBStateV2, fs: FSStateV2) -> Self {
        Self {
            db,
            fs,
            // agents: AgentsStateV2::new(),
        }
    }
}
