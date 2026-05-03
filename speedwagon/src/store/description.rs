//! KB-level description for a routing agent. Built from each doc's `purpose`
//! (titles are kept for the fallback). Output stays self-contained — no
//! mention of peer KBs — so a renamed or removed neighbor cannot stale it.
//! Korean output lands ~1/3 the chars of English at the same budget;
//! per-language budgets are deferred until a near-domain Korean KB shows up.

use ailoy::{
    agent::{Agent, AgentSpec},
    message::{Message, Part, Role},
};
use anyhow::Result;
use futures::StreamExt as _;

const MODEL: &str = "openai/gpt-5.4-mini";

const DESCRIPTION_INSTRUCTION: &str = concat!(
    "You write a self-contained description of a knowledge base. ",
    "This description will be read by a routing agent that picks the right ",
    "knowledge base for a user's question. ",
    "Inputs: KB name, optional instruction, and a list of one-line document purposes. ",
    "Describe what is INSIDE this knowledge base — its document types, ",
    "the entities and time periods covered, and the topics it can answer. ",
    "Lead with the collective identity of the documents. ",
    "Describe this KB on its own terms — do not compare it to other KBs, ",
    "list what it excludes, or mention neighboring KB names. ",
    "Output must NOT mention dataset names, QA pairs, paper IDs, contract IDs, ",
    "or any metadata about how this knowledge base was assembled. ",
    "Describe ONLY what documents are inside, as if a curator wrote it. ",
    "Write the description in English regardless of the document language. ",
    "Length: ~200 characters. Output a JSON object: {\"description\": \"<text>\"}."
);

/// Uses ailoy's process-global default provider (see `default_provider_mut`).
pub struct DescriptionAgent {
    spec: AgentSpec,
}

impl Default for DescriptionAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl DescriptionAgent {
    pub fn new() -> Self {
        Self {
            spec: AgentSpec::new(MODEL).instruction(DESCRIPTION_INSTRUCTION),
        }
    }

    /// Only `purpose` is sent to the LLM; `title` is kept in the signature
    /// so the caller can feed the same slice to `fallback_description`.
    pub async fn generate(
        &self,
        kb_name: &str,
        instruction: Option<&str>,
        docs: &[(&str, &str)],
    ) -> Result<String> {
        let user = build_user_message(kb_name, instruction, docs);
        let query = Message::new(Role::User).with_contents([Part::text(user)]);

        let mut agent = Agent::try_new(self.spec.clone()).await?;

        let mut text_parts: Vec<String> = Vec::new();
        {
            let mut stream = agent.run(query);
            while let Some(result) = stream.next().await {
                let output = result?;
                for part in &output.message.contents {
                    if let Some(text) = part.as_text() {
                        text_parts.push(text.to_string());
                    }
                }
            }
        }
        let raw = text_parts.join("");
        Ok(parse_description_response(&raw))
    }
}

fn build_user_message(
    kb_name: &str,
    instruction: Option<&str>,
    docs: &[(&str, &str)],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("KB name: {kb_name}\n"));
    if let Some(instr) = instruction {
        if !instr.trim().is_empty() {
            s.push_str(&format!("KB instruction: {instr}\n"));
        }
    }
    s.push_str(&format!("\nDocuments ({}):\n", docs.len()));
    for (_title, purpose) in docs {
        let p = if purpose.is_empty() { "(no purpose)" } else { *purpose };
        s.push_str(&format!("- {p}\n"));
    }
    s
}

/// Empty return signals the caller to use `fallback_description`.
fn parse_description_response(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(d) = value.get("description").and_then(|v| v.as_str()) {
            return d.trim().to_string();
        }
    }

    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]) {
                if let Some(d) = value.get("description").and_then(|v| v.as_str()) {
                    return d.trim().to_string();
                }
            }
        }
    }

    String::new()
}

/// Runs `DescriptionAgent` and substitutes `fallback_description` if the LLM
/// body is empty. Transport errors propagate.
pub async fn get_description(
    kb_name: &str,
    instruction: Option<&str>,
    docs: &[(&str, &str)],
) -> Result<String> {
    let result = DescriptionAgent::new()
        .generate(kb_name, instruction, docs)
        .await?;
    if result.is_empty() {
        log::warn!("description generation returned empty string; using fallback");
        let titles: Vec<&str> = docs.iter().map(|(t, _)| *t).collect();
        Ok(fallback_description(docs.len(), &titles))
    } else {
        Ok(result)
    }
}

/// Deterministic fallback when the LLM call fails or returns empty.
pub fn fallback_description(doc_count: usize, top_titles: &[&str]) -> String {
    if doc_count == 0 {
        return String::new();
    }
    let titles: Vec<&str> = top_titles
        .iter()
        .filter(|t| !t.is_empty())
        .take(5)
        .copied()
        .collect();
    if titles.is_empty() {
        format!("{doc_count} documents")
    } else {
        format!("{doc_count} documents including: {}", titles.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_direct_json() {
        let raw = r#"{"description": "hello"}"#;
        assert_eq!(parse_description_response(raw), "hello");
    }

    #[test]
    fn parse_json_with_surrounding_text() {
        let raw = r#"Here you go: {"description": "hello"} done."#;
        assert_eq!(parse_description_response(raw), "hello");
    }

    #[test]
    fn parse_trims_inner_whitespace() {
        let raw = r#"{"description": "   hello   "}"#;
        assert_eq!(parse_description_response(raw), "hello");
    }

    #[test]
    fn parse_empty_input() {
        assert_eq!(parse_description_response(""), "");
        assert_eq!(parse_description_response("   "), "");
    }

    #[test]
    fn parse_missing_field() {
        let raw = r#"{"other": "value"}"#;
        assert_eq!(parse_description_response(raw), "");
    }

    #[test]
    fn parse_malformed_json() {
        assert_eq!(parse_description_response("{not json"), "");
    }

    #[test]
    fn fallback_zero_docs() {
        assert_eq!(fallback_description(0, &[]), "");
    }

    #[test]
    fn fallback_no_titles_falls_back_to_count_only() {
        assert_eq!(fallback_description(7, &[]), "7 documents");
        assert_eq!(fallback_description(7, &["", ""]), "7 documents");
    }

    #[test]
    fn fallback_with_titles_takes_top_five() {
        let titles = ["A", "B", "C", "D", "E", "F", "G"];
        assert_eq!(
            fallback_description(7, &titles),
            "7 documents including: A; B; C; D; E"
        );
    }

    #[test]
    fn user_message_skips_blank_instruction() {
        let docs = [("T1", "P1"), ("T2", "")];
        let msg = build_user_message("kb1", Some("   "), &docs);
        assert!(msg.contains("KB name: kb1"));
        assert!(!msg.contains("KB instruction"));
        assert!(msg.contains("- P1"));
        assert!(msg.contains("- (no purpose)"));
    }

    #[test]
    fn user_message_renders_purpose_only_not_title() {
        let docs = [("Apple 10-K", "Apple FY2021 annual report")];
        let msg = build_user_message("finance", None, &docs);
        // purpose is in, title is not
        assert!(msg.contains("Apple FY2021 annual report"));
        assert!(!msg.contains("Apple 10-K"));
    }

    // ---- Store integration ----

    #[tokio::test]
    async fn describe_returns_empty_for_empty_store() {
        // Empty store → describe short-circuits before any LLM call,
        // so this runs without OPENAI_API_KEY.
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = crate::store::Store::new(tmp.path()).expect("open store");
        let result = store
            .describe("kb-empty", None)
            .await
            .expect("describe should not fail on empty store");
        assert_eq!(result, "");
    }

    #[tokio::test]
    #[ignore = "requires OPENAI_API_KEY"]
    async fn describe_round_trips_against_pre_populated_index() {
        // Pre-populate the index with deterministic (title, purpose) pairs
        // so this test only exercises the description path, not PurposeAgent
        // / TitleAgent. Mirrors the indexer round-trip tests in spirit.
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let index_dir = root.join("index");
        let index = crate::store::indexer::open_or_create(&index_dir).expect("open index");
        crate::store::indexer::add_document(
            &index,
            "doc1",
            "Apple FY2021 10-K",
            "Apple Inc. 2021 Form 10-K annual report — revenue, iPhone, services, SEC filing",
            "body 1",
        )
        .expect("add doc1");
        crate::store::indexer::add_document(
            &index,
            "doc2",
            "Walmart FY2023 10-K",
            "Walmart Inc. FY2023 Form 10-K annual report — omnichannel, retail, SEC filing",
            "body 2",
        )
        .expect("add doc2");
        // drop the writer-backed Index so Store::new can reopen the same dir.
        drop(index);

        // We need origin/ and corpus/ to exist for Store::new.
        std::fs::create_dir_all(root.join("origin")).unwrap();
        std::fs::create_dir_all(root.join("corpus")).unwrap();

        // Populate ailoy's process-global default provider for this test.
        dotenvy::dotenv().ok();
        ailoy::agent::default_provider_mut().await.model_openai(
            std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY required for this test"),
        );

        let store = crate::store::Store::new(root).expect("open store");
        let description = store
            .describe("finance", Some("public-company financial filings"))
            .await
            .expect("describe");

        assert!(!description.is_empty(), "description should not be empty");
        assert!(
            description.chars().count() < 600,
            "description should stay short: {description:?}"
        );
        // The model should pick up at least one of the entity tokens we fed in.
        let lowered = description.to_lowercase();
        let mentions_finance_token = ["apple", "walmart", "10-k", "filing", "financial"]
            .iter()
            .any(|t| lowered.contains(t));
        assert!(
            mentions_finance_token,
            "description did not mention any expected token: {description:?}"
        );
    }
}
