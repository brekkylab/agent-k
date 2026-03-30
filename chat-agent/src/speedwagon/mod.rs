//! Speedwagon sub-agent integration.
//!
//! Exposes the `ask_speedwagon` tool to the parent ChatAgent — a short-lived
//! sub-agent that queries a pre-indexed knowledge base.
//!
//! Each invocation is stateless — the sub-agent has no memory of prior calls.

pub mod dispatch;

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub use dispatch::{build_speedwagon_tool, dispatch_speedwagon};

const KNOWLEDGE_AGENTS_CONFIG: &str = "./data/knowledge_agents.json";
pub const ASK_SPEEDWAGON_TOOL: &str = "ask_speedwagon";

#[derive(Debug, Clone, Deserialize)]
pub struct KbEntry {
    pub id: String,
    pub description: String,
    pub index_dir: String,
    pub corpus_dirs: Vec<String>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub lm: Option<String>,
}

/// Provider config extracted from the parent ChatAgent's provider,
/// passed down to speedwagon sub-agents so they use the same API credentials.
#[derive(Clone)]
pub struct SubAgentProvider {
    pub api_key: String,
    pub api_url: String,
    /// The parent agent's model name, used as fallback when KbEntry.lm is None.
    pub model_name: String,
}

/// Load KB configuration from `KNOWLEDGE_AGENTS_CONFIG` env var or `./data/knowledge_agents.json`.
/// Relative paths in `index_dir` and `corpus_dirs` are resolved against the
/// directory containing the JSON config file, not the process CWD.
/// Returns empty Vec if file is missing or unparseable.
pub fn load_kb_config_from_file() -> Vec<KbEntry> {
    let path = PathBuf::from(
        std::env::var("KNOWLEDGE_AGENTS_CONFIG")
            .unwrap_or_else(|_| KNOWLEDGE_AGENTS_CONFIG.to_string()),
    );
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[knowledge] config not found at {}: {e}", path.display());
            return vec![];
        }
    };
    let base_dir = path
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let mut entries: Vec<KbEntry> = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[knowledge] failed to parse {}: {e}", path.display());
            return vec![];
        }
    };
    for entry in &mut entries {
        entry.index_dir = resolve_path(&base_dir, &entry.index_dir);
        entry.corpus_dirs = entry
            .corpus_dirs
            .iter()
            .map(|d| resolve_path(&base_dir, d))
            .collect();
    }
    entries
}

/// Resolve a path relative to a base directory. Absolute paths are returned as-is.
fn resolve_path(base: &Path, raw: &str) -> String {
    let p = Path::new(raw);
    if p.is_absolute() {
        raw.to_string()
    } else {
        base.join(p).to_string_lossy().to_string()
    }
}
