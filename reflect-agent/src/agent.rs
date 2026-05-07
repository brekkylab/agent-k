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
    datatype::Value,
    message::{Message, MessageOutput, Part, Role, ToolDescBuilder},
    tool::{ToolContext, ToolFactory, ToolFunc},
};
use anyhow::Result;
use futures::StreamExt as _;

use crate::reflect::{DEFAULT_REFLECT_MODEL, ReflectMode, ReflectVerdict, reflect_call};
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

/// System prompt fragment appended in [`ReflectMode::Self_`]. The
/// "MUST + do not skip" framing is the strongest of the three variants
/// we surveyed off-tree against operationally-realistic tasks (PyTorch
/// release listing, arXiv abstract summary, OSS DB release-date table,
/// ripgrep `Cargo.toml` parse) — it produced the highest verify-tool
/// invocation rate per turn. Even so, the call is best-effort: nothing
/// in the runtime *forces* it, so this mode remains self-discipline
/// driven and the verify-tool invocation rate falls off sharply on
/// trivial tasks where the LLM judges its draft as obviously correct.
const SELF_MODE_SYSTEM_PROMPT: &str = "\
You MUST call the `verify` tool with your draft answer before sending \
any final response. Do not skip this step. The verify tool returns a \
JSON verdict with `verdict` set to either \"stop\" or \"retry\". If \
`stop`, deliver your draft as the final answer. If `retry`, treat the \
returned `next_query` as a corrected task description and try again, \
calling `verify` again on the new draft. Limit yourself to one retry \
per user turn; on the second retry verdict, deliver whatever you have.";

/// Build the same agent as [`build_agent`], but with the reflect mode
/// applied. `Off` is identical to [`build_agent`]; `Self_` registers a
/// `verify` tool and prepends the self-mode instruction; `Forced` builds
/// the plain agent (the wrapper is applied at run time, not build time).
pub async fn build_agent_with_mode(model: &str, mode: ReflectMode) -> Result<Agent> {
    let mut provider = default_provider_mut().await.clone();
    attach_default_tools(&mut provider);

    let mut spec_instruction: Option<String> = None;

    if mode == ReflectMode::Self_ {
        // Tool: register the `verify` tool source on the provider so the
        // builder can resolve it by key below.
        provider.tools = std::mem::take(&mut provider.tools)
            .custom(verify_tool_factory(provider.clone(), DEFAULT_REFLECT_MODEL.to_string()));
        spec_instruction = Some(SELF_MODE_SYSTEM_PROMPT.to_string());
    }

    let mut builder = AgentBuilder::new(model)
        .provider(provider)
        .tool("bash")
        .tool("python_repl")
        .tool("web_search");
    if mode == ReflectMode::Self_ {
        builder = builder.tool("verify");
    }
    if let Some(inst) = spec_instruction {
        builder = builder.instruction(inst);
    }
    builder.build().await
}

/// Outcome of one [`run_with_forced_reflect`] call. Carries everything the
/// CLI / a test wants to inspect: the agent outputs (one or two turns
/// worth, depending on whether a retry fired), the deterministic verify
/// report, and the chain of reflect verdicts that drove the retry loop.
#[derive(Debug)]
pub struct ForcedReflectOutcome {
    pub outputs: Vec<MessageOutput>,
    pub verify_report: VerifyReport,
    pub reflect_verdicts: Vec<ReflectVerdict>,
    pub retry_count: usize,
}

/// Forced-mode wrapper around [`Agent::run`]. Drives one user turn to
/// completion, runs [`reflect_call`] on the draft answer, and — if the
/// verdict is `Retry` and the budget hasn't been spent — re-invokes the
/// agent with the verifier's `next_query`. The deterministic verify pass
/// from PR #53 also runs internally so the caller can compare what each
/// layer fired on without having to invoke them separately.
///
/// The retry budget is fixed at one per outer call. The second `Retry`
/// verdict (or a malformed one that coerces to `Stop`) terminates the
/// loop with whatever draft the agent produced last.
pub async fn run_with_forced_reflect(
    agent: &mut Agent,
    initial_query: Message,
    verify_config: &VerifyConfig,
    provider: &AgentProvider,
    reflect_model: &str,
) -> Result<ForcedReflectOutcome> {
    use crate::reflect::RETRY_BUDGET;

    let history_before = agent.get_history().len();
    let mut outputs: Vec<MessageOutput> = Vec::new();
    let mut verdicts: Vec<ReflectVerdict> = Vec::new();
    let mut retry_count = 0usize;
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

        let verdict = reflect_call(provider, reflect_model, &draft, &[]).await?;
        verdicts.push(verdict.clone());

        match verdict {
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
    })
}

/// Outcome of one [`run_with_hybrid`] call. Like [`ForcedReflectOutcome`]
/// plus per-turn verify reports — Hybrid mode runs the deterministic
/// verifier on the slice that each agent turn produced (not just the
/// final one), so a retry triggered by the verifier's hints can be
/// inspected against the issues the verifier saw at that point.
#[derive(Debug)]
pub struct HybridReflectOutcome {
    pub outputs: Vec<MessageOutput>,
    /// One report per agent turn, oldest first. `len()` is `1 + retry_count`.
    pub per_turn_verify_reports: Vec<VerifyReport>,
    /// Final-state verify report computed over the whole new history slice.
    pub verify_report: VerifyReport,
    pub reflect_verdicts: Vec<ReflectVerdict>,
    pub retry_count: usize,
    /// Set when at least one `Stop` verdict was promoted to a retry because
    /// its `confidence` was below [`crate::reflect::HYBRID_LOW_CONFIDENCE_THRESHOLD`].
    /// Useful in the CLI's verbose output and in tests that want to assert
    /// the threshold is wired up correctly.
    pub low_confidence_promotions: usize,
}

/// Hybrid wrapper. Runs the agent, verifies its output deterministically,
/// and feeds the verifier's findings into the reflect call as hints —
/// the LLM gets to read the deterministic flags before deciding `stop`
/// or `retry`. Adds one rule on top of [`run_with_forced_reflect`]: a
/// `Stop` verdict whose `confidence` is below
/// [`crate::reflect::HYBRID_LOW_CONFIDENCE_THRESHOLD`] is promoted to a
/// retry within the standard retry budget. The retry's `next_query`
/// comes from the verifier itself ("you said stop with low confidence
/// — try the same task again"), so we don't strand the loop.
///
/// On `Retry` the wrapper combines the verifier's `next_query` with the
/// freshest set of deterministic hints, keeping the next attempt aware
/// of what the previous one tripped.
pub async fn run_with_hybrid(
    agent: &mut Agent,
    initial_query: Message,
    verify_config: &VerifyConfig,
    provider: &AgentProvider,
    reflect_model: &str,
) -> Result<HybridReflectOutcome> {
    use crate::reflect::{HYBRID_LOW_CONFIDENCE_THRESHOLD, RETRY_BUDGET};

    let history_before = agent.get_history().len();
    let mut outputs: Vec<MessageOutput> = Vec::new();
    let mut per_turn_reports: Vec<VerifyReport> = Vec::new();
    let mut verdicts: Vec<ReflectVerdict> = Vec::new();
    let mut retry_count = 0usize;
    let mut low_conf_promos = 0usize;
    let mut next_query = initial_query;

    loop {
        let turn_start = agent.get_history().len();

        let mut stream = agent.run(next_query);
        while let Some(item) = stream.next().await {
            outputs.push(item?);
        }
        drop(stream);

        // Verify what this single turn produced, not the whole accumulated
        // slice — Hybrid feeds *fresh* signals into each reflect call.
        let turn_report = verify_run(&agent.get_history()[turn_start..], verify_config);
        let hints = render_verify_hints(&turn_report);
        per_turn_reports.push(turn_report);

        let draft = match last_assistant_text(&agent.get_history()[history_before..]) {
            Some(t) => t,
            None => break,
        };

        let verdict = reflect_call(provider, reflect_model, &draft, &hints).await?;
        verdicts.push(verdict.clone());

        // Promote a low-confidence Stop into a synthetic Retry — the goal
        // is to give the agent one more shot rather than silently honour
        // an "I'm not sure" verdict.
        let effective_verdict = match &verdict {
            ReflectVerdict::Stop {
                confidence: Some(c),
                ..
            } if *c < HYBRID_LOW_CONFIDENCE_THRESHOLD && retry_count < RETRY_BUDGET => {
                low_conf_promos += 1;
                ReflectVerdict::Retry {
                    rationale: format!(
                        "verifier confidence {c:.2} below {HYBRID_LOW_CONFIDENCE_THRESHOLD:.2}; retrying"
                    ),
                    next_query: "Please reattempt the previous task more carefully — your last \
                                 draft scored below the confidence threshold."
                        .to_string(),
                    confidence: Some(*c),
                }
            }
            _ => verdict.clone(),
        };

        match effective_verdict {
            ReflectVerdict::Stop { .. } => break,
            ReflectVerdict::Retry { next_query: nq, .. } => {
                if retry_count >= RETRY_BUDGET {
                    break;
                }
                retry_count += 1;
                // Re-pose the task; verifier hints will be regenerated from
                // the next turn's slice when we re-enter the loop.
                next_query = Message::new(Role::User).with_contents([Part::text(nq)]);
            }
        }
    }

    let verify_report = verify_run(&agent.get_history()[history_before..], verify_config);
    Ok(HybridReflectOutcome {
        outputs,
        per_turn_verify_reports: per_turn_reports,
        verify_report,
        reflect_verdicts: verdicts,
        retry_count,
        low_confidence_promotions: low_conf_promos,
    })
}

/// Render a [`VerifyReport`]'s issues as the bullet-list strings the
/// reflect call expects. Empty report → empty vec, which means
/// `reflect_call` falls back to its baseline (no-hints) prompt.
fn render_verify_hints(report: &VerifyReport) -> Vec<String> {
    if report.is_empty() {
        return Vec::new();
    }
    report
        .format()
        .lines()
        .filter_map(|l| l.strip_prefix("- ").map(str::to_string))
        .collect()
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

/// Construct the `verify` [`ToolFactory`] used by [`ReflectMode::Self_`].
/// The returned factory closes over a snapshot of the provider so each
/// `verify` invocation can spin up its own [`ailoy::lang_model::LangModel`]
/// for the reflect call, independent of the main agent's model.
fn verify_tool_factory(provider: AgentProvider, reflect_model: String) -> ToolFactory {
    let desc = ToolDescBuilder::new("verify")
        .description(
            "Self-check your draft answer before delivering it to the user. \
             Returns a JSON object with `verdict` (\"stop\" or \"retry\"), \
             `rationale`, and (when retry) `next_query`. Always call this \
             once on your draft before emitting your final answer.",
        )
        .parameters(ailoy::to_value!({
            "type": "object",
            "properties": {
                "draft_answer": {
                    "type": "string",
                    "description": "Your draft final answer, exactly as you would send it to the user."
                }
            },
            "required": ["draft_answer"]
        }))
        .build();

    let func = ToolFunc::new(move |args: Value, _ctx: ToolContext| {
        let provider = provider.clone();
        let reflect_model = reflect_model.clone();
        async move {
            let draft = args
                .pointer("/draft_answer")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if draft.trim().is_empty() {
                return ailoy::to_value!({
                    "verdict": "stop",
                    "rationale": "verify called with empty draft_answer; coerced to stop",
                });
            }
            match reflect_call(&provider, &reflect_model, &draft, &[]).await {
                Ok(verdict) => verdict_to_tool_value(&verdict),
                Err(e) => ailoy::to_value!({
                    "verdict": "stop",
                    "rationale": format!("reflect_call failed: {e}; coerced to stop"),
                }),
            }
        }
    });

    ToolFactory::simple(desc, func)
}

/// Render a [`ReflectVerdict`] back into the JSON object the calling LLM
/// expects from the `verify` tool. `confidence` is included only when the
/// verifier produced a usable value — nothing leaks for fail-open or
/// out-of-range cases.
fn verdict_to_tool_value(v: &ReflectVerdict) -> ailoy::datatype::Value {
    match v {
        ReflectVerdict::Stop {
            rationale,
            confidence,
        } => match confidence {
            Some(c) => ailoy::to_value!({
                "verdict": "stop",
                "rationale": rationale.as_str(),
                "confidence": *c as f64,
            }),
            None => ailoy::to_value!({
                "verdict": "stop",
                "rationale": rationale.as_str(),
            }),
        },
        ReflectVerdict::Retry {
            rationale,
            next_query,
            confidence,
        } => match confidence {
            Some(c) => ailoy::to_value!({
                "verdict": "retry",
                "rationale": rationale.as_str(),
                "next_query": next_query.as_str(),
                "confidence": *c as f64,
            }),
            None => ailoy::to_value!({
                "verdict": "retry",
                "rationale": rationale.as_str(),
                "next_query": next_query.as_str(),
            }),
        },
    }
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

    /// `verify_tool_factory` should produce a [`ToolFactory`] whose name
    /// is `"verify"` and whose schema requires the `draft_answer` field.
    /// Doesn't run the closure (that needs an LLM); just inspects the
    /// resolved [`Tool`] descriptor.
    #[test]
    fn verify_tool_factory_describes_a_verify_tool() {
        let factory = verify_tool_factory(AgentProvider::new(), "test/model".to_string());
        assert_eq!(factory.get_name(), "verify");

        // Resolve to a Tool against an empty spec to inspect the desc.
        let spec = ailoy::agent::AgentSpec::new("test/model");
        let tool = factory.make(&spec);
        let desc = tool.get_desc();
        assert_eq!(desc.name, "verify");

        let required = desc
            .parameters
            .pointer("/required")
            .and_then(|v| v.as_array())
            .expect("schema should declare required[]");
        let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_names.contains(&"draft_answer"));
    }
}
