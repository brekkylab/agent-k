//! Single registration site for ailoy `LangModelProvider` entries from
//! environment variables.
//!
//! Note that `ailoy::agent::AgentProvider::new()` already populates the
//! default provider from `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` /
//! `GEMINI_API_KEY`, so this helper is largely redundant for the standard
//! patterns. It is kept on the public surface so callers can re-register
//! after mutating env, and to mirror the helper of the same name in
//! sibling crates.

use ailoy::{agent::AgentProvider, lang_model::LangModelProvider};

/// Read the conventional API-key environment variables and register matching
/// glob patterns on `provider.models`. Missing keys are silently skipped, so
/// callers can register only the providers they actually have credentials
/// for. Idempotent — re-registering a pattern overwrites the previous entry.
pub fn register_provider_from_env(provider: &mut AgentProvider) {
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        provider
            .models
            .insert("openai/*".into(), LangModelProvider::openai(key));
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        provider
            .models
            .insert("anthropic/*".into(), LangModelProvider::anthropic(key));
    }
    if let Ok(key) = std::env::var("GEMINI_API_KEY") {
        provider
            .models
            .insert("gemini/*".into(), LangModelProvider::gemini(key));
    }
}
