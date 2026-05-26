use std::path::PathBuf;

use ailoy::message::Message;

pub mod coworker;
pub mod deep_research;

pub use coworker::get_coworker_cases;
pub use deep_research::get_deep_research_cases;

pub struct Case {
    pub query: Message,
    pub files: Vec<(Vec<u8>, PathBuf)>,
    pub shared_files: Vec<(Vec<u8>, PathBuf)>,
}
