//! Generic LLM helper trait. Implement `HelperAgent` once per helper — the
//! trait supplies the dispatch boilerplate (provider selection, agent
//! construction, streaming, text concatenation, JSON-field extraction,
//! empty-response fallback) via default method bodies. All helpers read
//! from ailoy's process-global default provider; populate it once at app
//! boot via `ailoy::agent::default_provider_mut`.
//!
//! # Hidden contract
//!
//! Every helper's `INSTRUCTION` must instruct the LLM to emit a JSON object
//! of the form `{"result": "<string>"}`. The default `process` impl
//! extracts the `"result"` field. Helpers with non-string outputs override
//! `process` to use their own response shape.

use ailoy::{
    agent::{Agent, AgentSpec},
    message::Message,
};
use anyhow::Result;
use futures::StreamExt as _;

const RESULT_FIELD: &str = "result";

/// Default model preference list shared by all metadata-style helpers
/// (Title / Purpose / Description). Order = preference. The default
/// `generate` body picks the first one whose provider is registered in
/// ailoy's process-global default; if none are registered, the helper
/// falls back without making an LLM call.
const DEFAULT_HELPER_MODELS: &[&str] = &[
    "openai/gpt-5.4-mini",
    "anthropic/claude-haiku-4-5-20251001",
    "google/gemini-2.5-flash",
];

/// Per-helper definition. Implement this on a unit struct, then call
/// `MyAgent::generate(input).await` directly.
pub(super) trait HelperAgent {
    /// Caller-supplied input. Borrowed by `build_query` / `fallback`, so the
    /// type itself never needs to be `Copy` or `Clone`.
    type Input<'a>;
    /// Parsed response.
    type Output;

    /// Preferred + fallback model ids in order. The default `generate`
    /// body picks the first one whose provider is registered in ailoy's
    /// process-global default. If none of them are registered, the helper
    /// logs a warning and runs `Self::fallback` without invoking the LLM.
    /// Defaults to `DEFAULT_HELPER_MODELS` — override only if this helper
    /// needs a different preference order.
    const MODELS: &'static [&'static str] = DEFAULT_HELPER_MODELS;
    /// System instruction. Must direct the LLM to emit
    /// `{"result": "<string>"}` — see module-level docs for the contract.
    const INSTRUCTION: &'static str;

    /// Render `input` to a chat `Message` to send to the LLM.
    fn build_query(input: &Self::Input<'_>) -> Message;

    /// Validate and convert the joined raw LLM response into `Output`, or
    /// return `None` to signal that the response was unusable (empty /
    /// malformed) and `fallback` should kick in.
    ///
    /// Default impl extracts the `"result"` string field. Override for
    /// non-string outputs.
    fn process(llm_resp: &str) -> Option<Self::Output>
    where
        Self::Output: From<String>,
    {
        extract_string_field(llm_resp, RESULT_FIELD).map(Into::into)
    }

    /// Deterministic substitute when `process` returns `None` or no
    /// provider is registered for any of `MODELS`.
    fn fallback(input: &Self::Input<'_>) -> Self::Output;

    /// Dispatch a single LLM call against ailoy's process-global default
    /// provider, run the response through `process`, and substitute
    /// `fallback` (with a warning log) if `process` returns `None` or no
    /// provider is registered for any of `MODELS`.
    async fn generate(input: Self::Input<'_>) -> Result<Self::Output>
    where
        Self::Output: From<String>,
    {
        let chosen_model = {
            let provider = ailoy::agent::default_provider().await;
            Self::MODELS
                .iter()
                .copied()
                .find(|m| provider.get_model(m).is_some())
        };

        let model = match chosen_model {
            Some(m) => m,
            None => {
                let name = std::any::type_name::<Self>()
                    .rsplit("::")
                    .next()
                    .unwrap_or("?");
                log::warn!(
                    "{name}: no provider registered for any of {:?}; using fallback",
                    Self::MODELS
                );
                return Ok(Self::fallback(&input));
            }
        };

        let spec = AgentSpec::new(model).instruction(Self::INSTRUCTION);
        let mut agent = Agent::try_new(spec).await?;
        let query = Self::build_query(&input);

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
        let llm_resp = text_parts.join("");
        match Self::process(&llm_resp) {
            Some(v) => Ok(v),
            None => {
                let name = std::any::type_name::<Self>()
                    .rsplit("::")
                    .next()
                    .unwrap_or("?");
                log::warn!("{name} returned empty/malformed response; using fallback");
                Ok(Self::fallback(&input))
            }
        }
    }
}

/// Extract a string-valued field from a JSON-shaped LLM response. Tries
/// `serde_json` directly first; if that fails (the model added prose), it
/// falls back to extracting the `{...}` substring before parsing. Returns
/// `None` for empty / malformed / field-missing / empty-value cases.
fn extract_string_field(raw: &str, field: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(s) = value.get(field).and_then(|v| v.as_str())
    {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }

    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}'))
        && start < end
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end])
        && let Some(s) = value.get(field).and_then(|v| v.as_str())
    {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ailoy::message::{Part, Role};

    #[test]
    fn extract_field_plain_json() {
        assert_eq!(
            extract_string_field(r#"{"foo": "hello"}"#, "foo"),
            Some("hello".to_string())
        );
    }

    #[test]
    fn extract_field_with_whitespace() {
        assert_eq!(
            extract_string_field("\n  {\"foo\": \"hello\"}  \n", "foo"),
            Some("hello".to_string())
        );
    }

    #[test]
    fn extract_field_with_surrounding_text() {
        assert_eq!(
            extract_string_field(r#"Sure: {"foo": "hello"} done."#, "foo"),
            Some("hello".to_string())
        );
    }

    #[test]
    fn extract_field_trims_inner_whitespace() {
        assert_eq!(
            extract_string_field(r#"{"foo": "   hello   "}"#, "foo"),
            Some("hello".to_string())
        );
    }

    #[test]
    fn extract_field_empty_input() {
        assert_eq!(extract_string_field("", "foo"), None);
        assert_eq!(extract_string_field("   ", "foo"), None);
    }

    #[test]
    fn extract_field_invalid_json() {
        assert_eq!(extract_string_field("not json", "foo"), None);
        assert_eq!(extract_string_field("{not: json}", "foo"), None);
    }

    #[test]
    fn extract_field_missing_field() {
        assert_eq!(extract_string_field(r#"{"other": "value"}"#, "foo"), None);
    }

    #[test]
    fn extract_field_whitespace_only_value() {
        assert_eq!(extract_string_field(r#"{"foo": "   "}"#, "foo"), None);
    }

    /// A test-only helper that lists a bogus model id. ailoy's `get_model`
    /// glob never matches it, so the trait's `generate` body always falls
    /// back without making an LLM call — regardless of whatever else any
    /// other test has registered on the global default.
    struct UnreachableModelAgent;

    impl HelperAgent for UnreachableModelAgent {
        type Input<'a> = &'a str;
        type Output = String;
        const MODELS: &'static [&'static str] = &["nonexistent/never-registered"];
        const INSTRUCTION: &'static str = "test";

        fn build_query(_: &&str) -> Message {
            Message::new(Role::User).with_contents([Part::text("")])
        }

        fn fallback(_: &&str) -> String {
            "FALLBACK".to_string()
        }
    }

    #[tokio::test]
    async fn generate_falls_back_when_no_provider_registered() {
        let result = UnreachableModelAgent::generate("anything")
            .await
            .expect("generate should succeed via fallback");
        assert_eq!(result, "FALLBACK");
    }
}
