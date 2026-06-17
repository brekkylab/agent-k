use std::path::PathBuf;

use ailoy::message::Message;

pub mod coworker;
pub mod deep_research;
pub mod speedwagon;

pub use coworker::get_coworker_cases;
pub use deep_research::get_deep_research_cases;
pub use speedwagon::get_speedwagon_cases;

pub struct Case {
    pub query: Message,
    pub files: Vec<(Vec<u8>, PathBuf)>,
    pub shared_files: Vec<(Vec<u8>, PathBuf)>,
    /// Documents to index into the Speedwagon corpus (the `knowledge` folder).
    /// Distinct from `shared_files` (Coworker's shared workspace): when non-empty
    /// the harness builds a `SharedStore` from these and binds Speedwagon to it,
    /// either directly or as a `subagent_speedwagon` sub-agent of Coworker/DeepResearch.
    pub corpus_files: Vec<(Vec<u8>, PathBuf)>,
}
