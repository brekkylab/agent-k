//! Speedwagon sub-agent integration.
//!
//! Registers one in-memory `AgentRuntime` per KB entry as a `SubagentTool` on
//! the parent ChatAgent's `ToolSet`.  Each subagent is built upfront at
//! `ChatAgent` construction time and persists for the lifetime of the agent.

pub mod dispatch;
pub mod indexing;

use serde::Deserialize;
use url::Url;

pub use dispatch::register_speedwagon_subagents;

/// Sub-agent behavior spec — the "what" of a speedwagon sub-agent.
/// Mirrors `AgentSpec` for the main agent: model choice and instruction.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SubAgentSpec {
    /// Model name override for this knowledge base (e.g. `"claude-sonnet-4-6"`).
    /// Falls back to the parent agent's model when `None`.
    pub lm: Option<String>,
    /// Custom system prompt addition for this knowledge base.
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KbEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub index_dir: String,
    pub corpus_dirs: Vec<String>,
    /// Sub-agent behavior: model and instruction overrides.
    #[serde(default)]
    pub spec: SubAgentSpec,
    /// File names in the corpus directory, exposed to the LLM so it can judge
    /// whether a question relates to this knowledge base.
    #[serde(default)]
    pub document_names: Vec<String>,
}

/// Provider config extracted from the parent ChatAgent's provider,
/// passed down to speedwagon sub-agents so they use the same API credentials.
/// Pure connection details — the "how" of reaching an LLM API.
#[derive(Clone)]
pub struct SubAgentProvider {
    pub api_key: String,
    pub api_url: Url,
    /// API wire protocol (ChatCompletion, Anthropic, Gemini).
    pub schema: ailoy::LangModelAPISchema,
}

impl SubAgentProvider {
    /// Extract API credentials from the parent agent's provider.
    pub fn from_provider(provider: &ailoy::AgentProvider) -> Self {
        match &provider.lm {
            ailoy::LangModelProvider::API {
                schema,
                url,
                api_key,
            } => Self {
                api_key: api_key.clone().unwrap_or_default(),
                api_url: url.clone(),
                schema: schema.clone(),
            },
        }
    }
}
