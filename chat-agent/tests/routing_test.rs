//! Integration tests for knowledge-agent routing.
//!
//! Verifies that the LLM selects the correct `kb_id` when calling
//! the `ask_knowledge` tool. Uses `ChatAgent::tool_call_log()` to
//! inspect which tools were called with which arguments.
//!
//! Requirements:
//!   - OPENAI_API_KEY environment variable
//!   - Pre-built tantivy indexes under `backend/data/index/`
//!
//! Run:  cargo test --test routing_test -- --ignored --nocapture

use std::path::PathBuf;
use std::sync::Once;

use ailoy::{AgentProvider, AgentSpec, LangModelAPISchema, LangModelProvider};
use chat_agent::ChatAgent;

static INIT_KB_CONFIG: Once = Once::new();

/// Set KNOWLEDGE_AGENTS_CONFIG to the backend's knowledge_agents.json
/// relative to this crate's location, so tests work regardless of CWD.
/// Uses `Once` to ensure the env var is set exactly once, even under
/// parallel test execution.
fn ensure_kb_config() {
    INIT_KB_CONFIG.call_once(|| {
        if std::env::var("KNOWLEDGE_AGENTS_CONFIG").is_ok() {
            return;
        }
        // chat-agent/../backend/data/knowledge_agents.json
        let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "backend", "data", "knowledge_agents.json"]
            .iter()
            .collect();
        // SAFETY: called exactly once before any parallel tests read this var.
        unsafe {
            std::env::set_var("KNOWLEDGE_AGENTS_CONFIG", path.canonicalize().expect("knowledge_agents.json not found"));
        }
    });
}

fn create_agent(model: &str) -> ChatAgent {
    ensure_kb_config();
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");

    let spec = AgentSpec {
        lm: model.to_string(),
        instruction: None,
        tools: vec![],
    };

    let provider = AgentProvider {
        lm: LangModelProvider::API {
            schema: LangModelAPISchema::ChatCompletion,
            url: "https://api.openai.com/v1/chat/completions"
                .parse()
                .unwrap(),
            api_key: Some(api_key),
        },
        tools: vec![],
    };

    ChatAgent::new(spec, provider, vec![], vec![])
}

async fn assert_routes_to(query: &str, expected_kb: &str) {
    let mut agent = create_agent("gpt-4.1-mini");
    let result = agent.run_user_text(query).await;
    assert!(result.is_ok(), "run_user_text failed: {result:?}");

    let entry = agent
        .tool_call_log()
        .iter()
        .find(|e| e.tool == "ask_knowledge");

    let entry = entry.expect(&format!(
        "query={query:?} → ask_knowledge was never called"
    ));

    let kb_id = entry.args.get("kb_id").and_then(|v| v.as_str());
    assert_eq!(
        kb_id,
        Some(expected_kb),
        "query={query:?} → expected kb_id={expected_kb:?}, got {kb_id:?}"
    );

    // Verify the tool returned a result (not an error)
    let result = entry.result.as_ref().expect("tool result should be present");
    assert!(
        result.get("answer").is_some(),
        "query={query:?} → tool result missing 'answer': {result}"
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
    assert_routes_to(
        "Who is the protagonist of Pride and Prejudice?",
        "novel",
    )
    .await;
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
