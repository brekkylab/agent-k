//! Speedwagon dispatch — building the `ask_speedwagon` tool and executing sub-agent queries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error_value;

use ailoy::agent::ToolFunc;
use ailoy::{LangModelProvider, ToolDescBuilder, ToolRuntime, Value};
use knowledge_agent::{AgentConfig, SearchIndex, ToolConfig, build_agent, run_with_trace};
use super::{ASK_SPEEDWAGON_TOOL, KbEntry, SubAgentProvider};

/// Grounding rules injected into every Speedwagon sub-agent system prompt.
/// Prevents the sub-agent from reframing document content through its own
/// parametric knowledge — document corpus is always the authoritative source.
const GROUNDING_RULES: &str = r#"<grounding_rules>
Your primary source of truth is the document corpus you are searching.
When you find relevant content, report it as the authoritative answer.

You MAY add expert analysis or synthesis that builds ON TOP of what
the documents say — this adds value.

You MUST NOT let your training knowledge reframe, contradict, or override
what the documents contain. If a document contains information about "X",
report that information as-is — even if "X" also refers to something
entirely different in the outside world.

The user is asking about THEIR documents. Answer from THEIR documents.

Example:
  Document "aurora.txt" contains a short story about a cat named Aurora.
  User question: "Tell me about aurora"

  BAD (reframing through your own knowledge):
    "This document is not about the aurora borealis.
     Instead it contains a story about a cat..."

  GOOD (document content as the anchor):
    "According to aurora.txt, Aurora is a cat living in an alley.
     The story describes her daily adventures..."
</grounding_rules>"#;

/// Build the `ask_speedwagon` tool from KB entries.
/// `default_provider` carries the parent agent's API credentials.
/// `default_model_name` is the parent agent's model, used as fallback when `KbEntry.spec.lm` is `None`.
/// `kb_overrides` maps kb_id → per-KB SubAgentProvider for cross-provider support.
/// Returns `None` if entries is empty.
pub fn build_speedwagon_tool(
    entries: &[KbEntry],
    default_provider: SubAgentProvider,
    default_model_name: String,
    kb_overrides: HashMap<String, SubAgentProvider>,
) -> Option<(String, ToolRuntime)> {
    if entries.is_empty() {
        return None;
    }

    let desc = ask_speedwagon_desc(entries);
    let func = ask_speedwagon_func(entries.to_vec(), default_provider, default_model_name, kb_overrides);
    Some((
        ASK_SPEEDWAGON_TOOL.to_string(),
        ToolRuntime::new(desc, func),
    ))
}

fn ask_speedwagon_desc(entries: &[KbEntry]) -> ailoy::ToolDesc {
    let kb_list = entries
        .iter()
        .map(|e| format!("- \"{}\": {}", e.id, e.description))
        .collect::<Vec<_>>()
        .join("\n");

    ToolDescBuilder::new(ASK_SPEEDWAGON_TOOL)
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

/// Returns a closure that spawns a speedwagon sub-agent per invocation.
/// Looks up `kb_overrides` first; falls back to `default_provider`.
fn ask_speedwagon_func(
    entries: Vec<KbEntry>,
    default_provider: SubAgentProvider,
    default_model_name: String,
    kb_overrides: HashMap<String, SubAgentProvider>,
) -> Arc<ToolFunc> {
    Arc::new(move |args: Value| {
        let entries = entries.clone();
        let default_provider = default_provider.clone();
        let default_model_name = default_model_name.clone();
        let kb_overrides = kb_overrides.clone();
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

            // Per-KB provider override; fall back to session's main provider
            let provider = kb_overrides.get(&kb_id).unwrap_or(&default_provider);

            match dispatch_speedwagon(&entry, &question, &default_model_name, provider).await {
                Ok(answer) => Value::object([("answer", Value::string(answer))]),
                Err(e) => {
                    tracing::error!("[speedwagon] sub-agent error for kb={kb_id}: {e}");
                    error_value(&e.to_string())
                }
            }
        })
    })
}

/// Spawn a short-lived speedwagon sub-agent, run a single query, and return the answer.
/// Uses `KbEntry.spec.instruction` as system prompt override (falls back to AgentConfig default).
/// Uses `KbEntry.spec.lm` as model name override (falls back to `default_model_name`).
pub async fn dispatch_speedwagon(
    entry: &KbEntry,
    question: &str,
    default_model_name: &str,
    provider: &SubAgentProvider,
) -> anyhow::Result<String> {
    let index_path = Path::new(&entry.index_dir);
    let search_index = Arc::new(SearchIndex::open(index_path)?);

    let target_dirs: Vec<PathBuf> = entry.corpus_dirs.iter().map(PathBuf::from).collect();

    let default_config = AgentConfig::default();

    let system_prompt = match &entry.spec.instruction {
        Some(custom) if !custom.trim().is_empty() => format!(
            "{}\n\n{}\n\n<additional_instructions>\n{}\n</additional_instructions>",
            default_config.system_prompt,
            GROUNDING_RULES,
            custom.trim()
        ),
        _ => format!("{}\n\n{}", default_config.system_prompt, GROUNDING_RULES),
    };

    let agent_config = AgentConfig {
        provider: LangModelProvider::API {
            schema: provider.schema.clone(),
            url: provider.api_url.clone(),
            api_key: Some(provider.api_key.clone()),
        },
        system_prompt,
        model_name: entry
            .spec
            .lm
            .clone()
            .unwrap_or_else(|| default_model_name.to_string()),
    };
    let tool_config = ToolConfig::default();

    let mut sub_agent = build_agent(&agent_config, &tool_config, &search_index, target_dirs);

    let (answer, _steps) = run_with_trace(&mut sub_agent, question).await?;
    Ok(answer)
}

