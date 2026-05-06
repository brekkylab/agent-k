//! Single lead agent built through [`ailoy::agent::AgentBuilder`].
//!
//! The agent has three built-in tools wired through ailoy's tool registry:
//! - `bash` — shell command execution
//! - `python_repl` — short-lived Python REPL
//! - `web_search` — DuckDuckGo / Yandex aggregator
//!
//! Tools run on the agent's [`ailoy::runenv::RunEnv`]. The default is
//! [`ailoy::runenv::Local`] (host-native execution); when the `sandbox`
//! feature is enabled, the caller may pass an [`Arc<Sandbox>`] wrapper
//! through a future builder option.
//!
//! Construction is split in two so the verify gate (Phase 1) and reflect
//! gate (Phase 2) can be layered on top without changing the call site.

use ailoy::{
    agent::{Agent, AgentBuilder, AgentProvider, default_provider_mut},
    message::{Message, MessageOutput},
};
use anyhow::Result;
use futures::StreamExt as _;

use crate::verify::{VerifyConfig, VerifyReport, verify_run};

/// Default model. Anthropic Haiku — fast, cheap, suitable for an interactive lead.
pub const DEFAULT_MODEL: &str = "anthropic/claude-haiku-4-5-20251001";

/// Build a [`reflect-agent`](crate) main agent on top of the process-global
/// [`ailoy::agent::default_provider`]. Caller is responsible for populating
/// the default provider with API keys before this is called — typically via
/// [`crate::register_provider_from_env`] at app boot.
///
/// `model` follows ailoy's `<provider>/<model-id>` convention (e.g.
/// `"anthropic/claude-haiku-4-5-20251001"`, `"openai/gpt-4o-mini"`).
pub async fn build_agent(model: &str) -> Result<Agent> {
    let mut provider = default_provider_mut().await.clone();
    attach_default_tools(&mut provider);

    AgentBuilder::new(model)
        .provider(provider)
        .tool("bash")
        .tool("python_repl")
        .tool("web_search")
        .build()
        .await
}

/// Register the three default builtin tools on `provider.tools`. Mutates
/// in place so the caller can layer additional tools (e.g. subagents in a
/// future PR) before handing the provider to the builder.
fn attach_default_tools(provider: &mut AgentProvider) {
    let mut tools = std::mem::take(&mut provider.tools);
    tools = tools.bash().python_repl().web_search();
    provider.tools = tools;
}

/// Stream one user turn, collect every [`MessageOutput`] the agent emits,
/// and run the post-hoc verify pass on the slice of history that this turn
/// just appended.
///
/// This is the Stage A entry point for the verify gate: the agent has
/// already finished and produced a response by the time the report is
/// built. Stage B (a future PR) will move signal evaluation between turns
/// via an ailoy-side hook so the same checks can intercept tool results.
pub async fn run_with_verify(
    agent: &mut Agent,
    query: Message,
    config: &VerifyConfig,
) -> Result<(Vec<MessageOutput>, VerifyReport)> {
    let history_before = agent.get_history().len();

    let mut outputs = Vec::new();
    let mut stream = agent.run(query);
    while let Some(item) = stream.next().await {
        outputs.push(item?);
    }
    drop(stream);

    let report = verify_run(&agent.get_history()[history_before..], config);
    Ok((outputs, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ailoy::tool::ToolProvider;

    /// `attach_default_tools` should add three [`ailoy::tool::ToolProviderElem`] entries.
    /// We don't introspect the concrete elements (private to ailoy's enum), so the
    /// check is by count via the public `iter()` interface.
    #[test]
    fn attach_default_tools_adds_three_entries() {
        let mut provider = AgentProvider::new();
        attach_default_tools(&mut provider);
        assert_eq!(provider.tools.iter().count(), 3);
    }

    /// Sanity: starting from a populated provider, `attach_default_tools` keeps
    /// the existing entries (it appends, doesn't replace).
    #[test]
    fn attach_default_tools_appends_to_existing() {
        let mut provider = AgentProvider::new();
        provider.tools = ToolProvider::new().bash();
        attach_default_tools(&mut provider);
        // 1 (initial bash) + 3 (default tools) = 4
        assert_eq!(provider.tools.iter().count(), 4);
    }
}
