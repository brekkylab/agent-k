//! Single registration site for ailoy `LangModelProvider` entries from
//! environment variables. Mirrors `speedwagon::register_provider_from_env`
//! (kept duplicated for now to avoid pulling speedwagon's full dependency
//! tree just for a 12-line helper).

use ailoy::{
    agent::AgentProvider,
    lang_model::{LangModelAPISchema, LangModelProviderElem},
};
use url::Url;

/// Read the conventional API-key environment variables and register matching
/// glob patterns on `provider.models`. Missing keys are silently skipped, so
/// callers can register only the providers they actually have credentials for.
///
/// Patterns and URLs match the helper constructors that ailoy used to expose
/// (`model_openai`, `model_claude`, `model_gemini`) before they were removed
/// in the post-#391 registry refactor.
pub fn register_provider_from_env(provider: &mut AgentProvider) {
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        provider.models.insert(
            "openai/*".into(),
            LangModelProviderElem::API {
                schema: LangModelAPISchema::OpenAI,
                url: Url::parse("https://api.openai.com/v1/responses").unwrap(),
                api_key: Some(key),
                max_tokens: None,
            },
        );
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        provider.models.insert(
            "anthropic/claude-*".into(),
            LangModelProviderElem::API {
                schema: LangModelAPISchema::Anthropic,
                url: Url::parse("https://api.anthropic.com/v1/messages").unwrap(),
                api_key: Some(key),
                max_tokens: None,
            },
        );
    }
    if let Ok(key) = std::env::var("GEMINI_API_KEY") {
        provider.models.insert(
            "google/gemini-*".into(),
            LangModelProviderElem::API {
                schema: LangModelAPISchema::Gemini,
                url: Url::parse("https://generativelanguage.googleapis.com/v1beta/models/")
                    .unwrap(),
                api_key: Some(key),
                max_tokens: None,
            },
        );
    }
}
