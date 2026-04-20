//! Integration tests for speedwagon sub-agent routing.
//!
//! Verifies that the LLM picks the correct per-KB `ask_speedwagon_<id>` tool.
//! Uses `ChatAgent::tool_call_log()` to inspect which tools were called.
//!
//! Requirements:
//!   - OPENAI_API_KEY environment variable
//!   - Pre-built tantivy indexes under `backend/data/index/{finance,novel}/`
//!   - `backend/data/knowledge_agents.json` describing the KBs
//!
//! Run:  cargo test --test routing_test -- --ignored --nocapture

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use ailoy::{AgentProvider, AgentSpec, LangModelProvider};
use chat_agent::{ChatAgent, KbEntry};
use serde::Deserialize;

#[derive(Deserialize)]
struct RawKbEntry {
    id: String,
    description: String,
    index_dir: String,
    corpus_dirs: Vec<String>,
}

fn load_kb_entries() -> Vec<KbEntry> {
    let config_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "backend",
        "data",
        "knowledge_agents.json",
    ]
    .iter()
    .collect();
    let config_path = config_path
        .canonicalize()
        .expect("knowledge_agents.json not found; run scripts/setup-data.sh first");
    let base_dir = config_path
        .parent()
        .expect("config path has no parent")
        .to_path_buf();

    let raw = fs::read_to_string(&config_path).expect("read knowledge_agents.json");
    let entries: Vec<RawKbEntry> =
        serde_json::from_str(&raw).expect("parse knowledge_agents.json");

    entries
        .into_iter()
        .map(|r| {
            let resolve = |p: &str| base_dir.join(p).to_string_lossy().into_owned();
            KbEntry {
                id: r.id.clone(),
                name: r.id.clone(),
                description: r.description,
                index_dir: resolve(&r.index_dir),
                corpus_dirs: r.corpus_dirs.iter().map(|p| resolve(p)).collect(),
                spec: Default::default(),
                document_names: vec![],
            }
        })
        .collect()
}

fn create_agent(model: &str) -> ChatAgent {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");

    let spec = AgentSpec {
        lm: model.to_string(),
        instruction: None,
        tools: vec![],
    };

    let provider = AgentProvider {
        lm: LangModelProvider::openai(api_key),
        tools: vec![],
    };

    ChatAgent::new(
        spec,
        provider,
        load_kb_entries(),
        HashMap::new(),
        vec![],
    )
}

async fn assert_routes_to(query: &str, expected_kb: &str) {
    let mut agent = create_agent("gpt-4.1-mini");
    let result = agent.run_user_text(query).await;
    assert!(result.is_ok(), "run_user_text failed: {result:?}");

    let expected_tool = format!("ask_speedwagon_{expected_kb}");
    let called: Vec<&str> = agent
        .tool_call_log()
        .iter()
        .map(|e| e.tool.as_str())
        .collect();
    assert!(
        called.contains(&expected_tool.as_str()),
        "query={query:?} → expected tool {expected_tool:?} not in calls {called:?}"
    );
}

// ── Finance routing ──

#[tokio::test]
#[ignore] // requires OPENAI_API_KEY + indexes
async fn routes_revenue_question_to_finance() {
    assert_routes_to("What was Apple's total revenue in 2022?", "finance").await;
}

#[tokio::test]
#[ignore]
async fn routes_expense_question_to_finance() {
    assert_routes_to("How much did Amazon spend on R&D in 2021?", "finance").await;
}

#[tokio::test]
#[ignore]
async fn routes_profit_question_to_finance() {
    assert_routes_to(
        "What was Microsoft's operating profit margin in 2020?",
        "finance",
    )
    .await;
}

// ── Novel routing ──

#[tokio::test]
#[ignore]
async fn routes_character_question_to_novel() {
    assert_routes_to("Who is the protagonist of Pride and Prejudice?", "novel").await;
}

#[tokio::test]
#[ignore]
async fn routes_theme_question_to_novel() {
    assert_routes_to("What is the main theme of Anna Karenina?", "novel").await;
}

#[tokio::test]
#[ignore]
async fn routes_plot_question_to_novel() {
    assert_routes_to("How does Wuthering Heights end?", "novel").await;
}
