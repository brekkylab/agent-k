//! Single lead agent built through [`ailoy::agent::AgentBuilder`].
//!
//! The agent has three built-in tools wired through ailoy's tool registry:
//! - `bash` — shell command execution
//! - `python_repl` — short-lived Python REPL
//! - `web_search` — DuckDuckGo / Yandex aggregator
//!
//! Tools run on the agent's [`ailoy::runenv::RunEnv`]. The default is
//! [`ailoy::runenv::Local`] (host-native execution).
//!
//! Construction is split in two so the verify gate and reflect gate can
//! be layered on top without changing the call site.

use ailoy::{
    agent::{Agent, AgentBuilder, AgentProvider, default_provider},
    message::{Message, MessageOutput, Part, Role},
    to_value,
    tool::{ToolDesc, ToolDescBuilder},
};
use anyhow::Result;
use futures::StreamExt as _;

use crate::reflect::{LOW_CONFIDENCE_THRESHOLD, ReflectMode, ReflectVerdict, reflect_call};
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
    let provider = default_provider().clone();
    AgentBuilder::new(model)
        .provider(provider)
        .tool(bash_tool_desc())
        .tool(python_repl_tool_desc())
        .tool(web_search_tool_desc())
        .build()
}

/// `ToolDesc` for the built-in `bash` tool. Schema mirrors ailoy's internal
/// `get_bash_tool_desc` (which is `pub(crate)` inside ailoy and not directly
/// accessible from downstream crates).
fn bash_tool_desc() -> ToolDesc {
    ToolDescBuilder::new("bash")
        .description("Execute a shell command and return stdout/stderr/exit_code.")
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "cmd": {
                    "type": "string",
                    "description": "Shell command to execute (interpreted by sh -c)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds. 0 or omitted means no timeout."
                }
            },
            "required": ["cmd"]
        }))
        .build()
}

/// `ToolDesc` for the built-in `python_repl` tool. Schema mirrors ailoy's
/// internal `get_python_repl_tool_desc`.
fn python_repl_tool_desc() -> ToolDesc {
    ToolDescBuilder::new("python_repl")
        .description(
            "Execute a Python script and return stdout/stderr. \
             Use `pip_install` to install packages before execution.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Python code to execute"
                },
                "pip_install": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Packages to install before running (e.g. 'numpy>=1.24')."
                }
            },
            "required": ["code"]
        }))
        .build()
}

/// `ToolDesc` for the built-in `web_search` tool. Schema mirrors ailoy's
/// internal `get_web_search_tool_desc`.
fn web_search_tool_desc() -> ToolDesc {
    ToolDescBuilder::new("web_search")
        .description(
            "Search the web using multiple search engines simultaneously. \
             Returns aggregated and deduplicated results ranked by how many \
             engines returned them. Use this tool to find current information, \
             facts, documentation, news, or any web-accessible content.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query string"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return. Default: 10. Max: 30.",
                    "default": 10
                }
            },
            "required": ["query"]
        }))
        .build()
}

/// Stream one user turn, collect every [`MessageOutput`] the agent emits,
/// and run the post-hoc verify pass on the slice of history that this turn
/// just appended. The agent has already produced its response by the time
/// the report is built; the gate only flags issues, it never intercepts.
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

/// Build the same agent as [`build_agent`], but with the reflect mode
/// applied. Both `Off` and `Forced` build the same plain agent — Forced's
/// reflect wrapper is applied at run time by [`run_with_forced_reflect`],
/// not at build time. The function is kept on the public surface so the
/// CLI can dispatch on mode without leaking the construction detail.
pub async fn build_agent_with_mode(model: &str, _mode: ReflectMode) -> Result<Agent> {
    build_agent(model).await
}

/// Outcome of one [`run_with_forced_reflect`] call. Carries everything
/// the CLI or a test wants to inspect: the agent outputs (one or two
/// turns worth depending on whether a retry fired), the deterministic
/// verify report, the verdict chain (one Haiku verdict per turn plus the
/// optional Sonnet escalation verdict), the retry count, and the number
/// of escalations to the stronger model.
#[derive(Debug)]
pub struct ForcedReflectOutcome {
    pub outputs: Vec<MessageOutput>,
    pub verify_report: VerifyReport,
    pub reflect_verdicts: Vec<ReflectVerdict>,
    pub retry_count: usize,
    /// How many times the first-pass verdict was a low-confidence `Stop`
    /// that triggered an escalation reflect call to the stronger model.
    /// `0` means the run stayed entirely on the first-pass model.
    pub escalations: usize,
}

/// Forced-mode wrapper around [`Agent::run`]. After each turn the wrapper
/// runs [`reflect_call`] on the draft with `reflect_model`. When the
/// first-pass verdict is a `Stop` whose confidence is below
/// [`LOW_CONFIDENCE_THRESHOLD`], the wrapper escalates: a second
/// `reflect_call` is made with `escalate_model` and that verdict becomes
/// final for this turn. The escalation model's `Stop` is honoured; its
/// `Retry` verdict is ignored, because the calibration data for this PR
/// found same-model retry never recovered the answer in those cases.
///
/// First-pass `Retry` verdicts are honoured with a one-attempt budget
/// ([`crate::reflect::RETRY_BUDGET`]). The second `Retry` verdict (or a
/// malformed one that coerces to `Stop`) terminates the loop with the
/// last draft.
pub async fn run_with_forced_reflect(
    agent: &mut Agent,
    initial_query: Message,
    verify_config: &VerifyConfig,
    provider: &AgentProvider,
    reflect_model: &str,
    escalate_model: &str,
) -> Result<ForcedReflectOutcome> {
    use crate::reflect::RETRY_BUDGET;

    let history_before = agent.get_history().len();
    let mut outputs: Vec<MessageOutput> = Vec::new();
    let mut verdicts: Vec<ReflectVerdict> = Vec::new();
    let mut retry_count = 0usize;
    let mut escalations = 0usize;
    let mut next_query = initial_query;

    loop {
        // Drive one full turn to completion.
        let mut stream = agent.run(next_query);
        while let Some(item) = stream.next().await {
            outputs.push(item?);
        }
        drop(stream);

        // Extract the draft from the final assistant message in the new
        // history slice. If the agent didn't emit an assistant message at
        // all, there's nothing to verify — break with whatever we have.
        let draft = match last_assistant_text(&agent.get_history()[history_before..]) {
            Some(t) => t,
            None => break,
        };

        // First-pass verdict from the cheap model.
        let first_verdict = reflect_call(provider, reflect_model, &draft, &[]).await?;
        verdicts.push(first_verdict.clone());

        // Decide whether this turn's effective verdict needs to come from
        // the stronger model: only when the first-pass verdict is a Stop
        // with reported confidence below the threshold. Retry verdicts
        // are passed through unchanged so the standard retry budget runs.
        let effective_verdict = match &first_verdict {
            ReflectVerdict::Stop {
                confidence: Some(c),
                ..
            } if *c < LOW_CONFIDENCE_THRESHOLD => {
                escalations += 1;
                let strong = reflect_call(provider, escalate_model, &draft, &[]).await?;
                verdicts.push(strong.clone());
                // Stronger-model Retry verdicts are ignored — see Q3 of the
                // PR's calibration report. We accept the original draft.
                match strong {
                    ReflectVerdict::Stop { .. } => strong,
                    ReflectVerdict::Retry { .. } => first_verdict,
                }
            }
            _ => first_verdict,
        };

        match effective_verdict {
            ReflectVerdict::Stop { .. } => break,
            ReflectVerdict::Retry { next_query: nq, .. } => {
                if retry_count >= RETRY_BUDGET {
                    // Budget spent — accept the last draft regardless.
                    break;
                }
                retry_count += 1;
                next_query = Message::new(Role::User).with_contents([Part::text(nq)]);
                // Loop continues: run a fresh turn with the verifier-supplied query.
            }
        }
    }

    let verify_report = verify_run(&agent.get_history()[history_before..], verify_config);
    Ok(ForcedReflectOutcome {
        outputs,
        verify_report,
        reflect_verdicts: verdicts,
        retry_count,
        escalations,
    })
}

fn last_assistant_text(history: &[Message]) -> Option<String> {
    let msg = history
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.contents.is_empty())?;
    let mut text = String::new();
    for part in &msg.contents {
        if let Some(t) = part.as_text() {
            text.push_str(t);
        }
    }
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three built-in tool descs we ship to the LLM should keep their
    /// canonical names — these are the lookup keys against the provider's
    /// pre-registered builtin tools.
    #[test]
    fn tool_descs_have_canonical_names() {
        assert_eq!(bash_tool_desc().name, "bash");
        assert_eq!(python_repl_tool_desc().name, "python_repl");
        assert_eq!(web_search_tool_desc().name, "web_search");
    }
}
