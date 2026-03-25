//! Knowledge sub-agent integration.
//!
//! Exposes a single tool (`ask_knowledge`) to the parent ChatAgent.
//! When the LLM calls this tool, a **short-lived sub-agent** is spawned:
//!
//!   1. A `knowledge_agent::AgentRuntime` is created with the selected KB's
//!      tantivy index and corpus.
//!   2. The sub-agent runs its own ReAct loop (search → find → open → answer).
//!   3. The answer is returned to the parent agent and the sub-agent is dropped.
//!
//! Each invocation is stateless — the sub-agent has no memory of prior calls.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ailoy::agent::ToolFunc;
use ailoy::{ToolDescBuilder, ToolRuntime, Value};
use knowledge_agent::{AgentConfig, SearchIndex, ToolConfig, build_agent, run_with_trace};
use serde::Deserialize;

const KNOWLEDGE_AGENTS_CONFIG: &str = "./data/knowledge_agents.json";
pub const ASK_KNOWLEDGE_TOOL: &str = "ask_knowledge";

#[derive(Debug, Clone, Deserialize)]
pub struct KbEntry {
    pub id: String,
    pub description: String,
    pub index_dir: String,
    pub corpus_dirs: Vec<String>,
}

/// Provider config extracted from the parent ChatAgent's provider,
/// passed down to knowledge sub-agents so they use the same API credentials.
#[derive(Clone)]
pub struct SubAgentProvider {
    pub api_key: String,
    pub api_url: String,
}

/// Load KB configuration from `KNOWLEDGE_AGENTS_CONFIG` env var or `./data/knowledge_agents.json`.
/// Relative paths in `index_dir` and `corpus_dirs` are resolved against the
/// directory containing the JSON config file, not the process CWD.
/// Returns empty Vec if file is missing or unparseable.
pub fn load_kb_config() -> Vec<KbEntry> {
    let path = PathBuf::from(
        std::env::var("KNOWLEDGE_AGENTS_CONFIG").unwrap_or_else(|_| KNOWLEDGE_AGENTS_CONFIG.to_string()),
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

/// Build the `ask_knowledge` tool from KB entries.
/// `provider` carries the parent agent's API credentials so sub-agents
/// use the same key instead of falling back to environment variables.
/// Returns `None` if entries is empty.
pub fn build_knowledge_tool(
    entries: &[KbEntry],
    provider: SubAgentProvider,
) -> Option<(String, ToolRuntime)> {
    if entries.is_empty() {
        return None;
    }

    let desc = ask_knowledge_desc(entries);
    let func = ask_knowledge_func(entries.to_vec(), provider);
    Some((ASK_KNOWLEDGE_TOOL.to_string(), ToolRuntime::new(desc, func)))
}

fn ask_knowledge_desc(entries: &[KbEntry]) -> ailoy::ToolDesc {
    let kb_list = entries
        .iter()
        .map(|e| format!("- \"{}\": {}", e.id, e.description))
        .collect::<Vec<_>>()
        .join("\n");

    ToolDescBuilder::new(ASK_KNOWLEDGE_TOOL)
        .description(format!(
            "Query a knowledge base to find answers from pre-indexed document corpora.\n\
             Available knowledge bases:\n{kb_list}"
        ))
        .parameters(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([
                    (
                        "kb_id",
                        Value::object([
                            ("type", Value::string("string")),
                            (
                                "description",
                                Value::string("ID of the knowledge base to query"),
                            ),
                            (
                                "enum",
                                Value::array(entries.iter().map(|e| Value::string(&e.id))),
                            ),
                        ]),
                    ),
                    (
                        "question",
                        Value::object([
                            ("type", Value::string("string")),
                            (
                                "description",
                                Value::string("The question to ask the knowledge base"),
                            ),
                        ]),
                    ),
                ]),
            ),
            (
                "required",
                Value::array([Value::string("kb_id"), Value::string("question")]),
            ),
        ]))
        .build()
}

/// Returns a closure that spawns a knowledge sub-agent per invocation.
/// The sub-agent is created, runs a single query, and is immediately dropped.
fn ask_knowledge_func(entries: Vec<KbEntry>, provider: SubAgentProvider) -> Arc<ToolFunc> {
    Arc::new(move |args: Value| {
        let entries = entries.clone();
        let provider = provider.clone();
        Box::pin(async move {
            let args_map = match args.as_object() {
                Some(m) => m,
                None => return error_value("invalid_arguments"),
            };

            let kb_id = match args_map.get("kb_id").and_then(Value::as_str) {
                Some(s) => s.to_string(),
                None => return error_value("missing kb_id"),
            };

            let question = match args_map.get("question").and_then(Value::as_str) {
                Some(s) => s.to_string(),
                None => return error_value("missing question"),
            };

            let entry = match entries.iter().find(|e| e.id == kb_id) {
                Some(e) => e.clone(),
                None => return error_value(&format!("unknown kb_id: {kb_id}")),
            };

            match spawn_sub_agent(&entry, &question, &provider).await {
                Ok(answer) => Value::object([("answer", Value::string(answer))]),
                Err(e) => {
                    eprintln!("[knowledge] sub-agent error for kb={kb_id}: {e}");
                    error_value(&e.to_string())
                }
            }
        })
    })
}

/// Spawn a short-lived knowledge sub-agent, run a single query, and return the answer.
/// The sub-agent (and its search index handle) are dropped when this function returns.
/// Uses the parent agent's API credentials via `provider`.
async fn spawn_sub_agent(
    entry: &KbEntry,
    question: &str,
    provider: &SubAgentProvider,
) -> anyhow::Result<String> {
    let index_path = Path::new(&entry.index_dir);
    let search_index = Arc::new(SearchIndex::open(index_path)?);

    let target_dirs: Vec<PathBuf> = entry.corpus_dirs.iter().map(PathBuf::from).collect();
    let agent_config = AgentConfig {
        api_key: provider.api_key.clone(),
        api_url: provider.api_url.clone(),
        ..AgentConfig::default()
    };
    let tool_config = ToolConfig::default();

    // Create sub-agent — it owns its own ReAct loop, independent of the parent ChatAgent
    let mut sub_agent = build_agent(
        &agent_config,
        &tool_config,
        &search_index,
        target_dirs,
    );

    let (answer, _steps) = run_with_trace(&mut sub_agent, question).await?;
    Ok(answer)
    // sub_agent is dropped here
}

fn error_value(msg: &str) -> Value {
    Value::object([("error", Value::string(msg))])
}
