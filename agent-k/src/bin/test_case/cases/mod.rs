use std::path::PathBuf;

use ailoy::message::Message;

pub mod coworker;
pub mod deep_research;
#[expect(dead_code, reason = "Speedwagon cases are staged for the upcoming harness mode")]
pub mod speedwagon;

pub use coworker::get_coworker_cases;
pub use deep_research::get_deep_research_cases;
#[expect(
    unused_imports,
    reason = "keeps the staged Speedwagon case API alongside the active modes"
)]
pub use speedwagon::get_speedwagon_cases;

pub struct Case {
    pub query: Message,
    pub files: Vec<(Vec<u8>, PathBuf)>,
    pub shared_files: Vec<(Vec<u8>, PathBuf)>,
    /// Documents to index into the Speedwagon corpus (the `knowledge` folder).
    /// Distinct from `shared_files` (Coworker's shared workspace): when non-empty
    /// the harness builds a `SharedStore` from these and binds Speedwagon to it,
    /// either directly or as a `subagent_speedwagon` sub-agent of Coworker/DeepResearch.
    #[expect(
        dead_code,
        reason = "corpus fixtures are staged for the upcoming Speedwagon harness mode"
    )]
    pub corpus_files: Vec<(Vec<u8>, PathBuf)>,
}
