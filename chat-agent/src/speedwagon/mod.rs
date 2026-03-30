//! Speedwagon sub-agent integration.
//!
//! Exposes the `ask_speedwagon` tool to the parent ChatAgent — a short-lived
//! sub-agent that queries a pre-indexed knowledge base.
//!
//! Each invocation is stateless — the sub-agent has no memory of prior calls.

pub mod dispatch;

use serde::Deserialize;

pub use dispatch::{build_speedwagon_tool, dispatch_speedwagon};

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

impl SubAgentProvider {
    /// Extract API credentials from the parent agent's provider.
    pub fn from_provider(provider: &ailoy::AgentProvider, model_name: &str) -> Self {
        match &provider.lm {
            ailoy::LangModelProvider::API { url, api_key, .. } => Self {
                api_key: api_key.clone().unwrap_or_default(),
                api_url: url.to_string(),
                model_name: model_name.to_string(),
            },
        }
    }
}

