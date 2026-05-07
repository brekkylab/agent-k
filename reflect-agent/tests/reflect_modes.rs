//! End-to-end comparison of `ReflectMode::{Off, Self_, Forced}` on the
//! demo task (unstructured experiment log → structured `(timestamp, metric,
//! value)` parse).
//!
//! All three tests are `#[ignore]`d because they require a live LLM API
//! key (`ANTHROPIC_API_KEY` preferred, `OPENAI_API_KEY` accepted). They
//! make no LLM-dependent assertions — the agent's exact output varies
//! run-to-run — but they do verify that each mode's pipeline runs to
//! completion and that the verify report / reflect verdicts surface in
//! the expected shape.
//!
//! Run all three:
//!
//! ```sh
//! ANTHROPIC_API_KEY=... cargo test -p reflect-agent --test reflect_modes \
//!     -- --ignored --nocapture
//! ```

use ailoy::{
    agent::{AgentProvider, default_provider, default_provider_mut},
    message::{Message, Part, Role},
};
use reflect_agent::{
    DEFAULT_REFLECT_MODEL, ReflectMode, VerifyConfig, build_agent_with_mode,
    register_provider_from_env, run_with_forced_reflect, run_with_hybrid, run_with_verify,
};

const LOG_FIXTURE: &str = "\
2024-01-15T10:30:00 cpu=42
2024-01-15T10:31:00 cpu=51
2024-01-15T10:32:00 cpu=47
";

const PROMPT_TEMPLATE: &str = "\
Below is an experiment log. Parse it into a list of \
(timestamp, metric, value) tuples using bash and python_repl. \
Print the parsed list — don't try to make a plot. Cite each row's \
timestamp from the log verbatim.

Log:
{log}";

/// Pick a model whose provider is registered. Anthropic is preferred for
/// stable tool-call behaviour; OpenAI is the fallback so this still runs
/// against an `OPENAI_API_KEY`-only environment.
fn pick_model() -> Option<&'static str> {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        Some("anthropic/claude-haiku-4-5-20251001")
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        Some("openai/gpt-4o-mini")
    } else {
        None
    }
}

fn build_query() -> Message {
    Message::new(Role::User).with_contents([Part::text(
        PROMPT_TEMPLATE.replace("{log}", LOG_FIXTURE),
    )])
}

async fn boot() -> Option<&'static str> {
    dotenvy::dotenv().ok();
    let model = pick_model()?;
    register_provider_from_env(&mut *default_provider_mut().await);
    Some(model)
}

/// Off mode reproduces PR #53's behaviour: no LLM-driven reflect, just
/// the deterministic verify pass at turn-end. We confirm the agent
/// produced an assistant message and the verify report rendered without
/// panicking — content-level assertions stay hands-off because LLM
/// behaviour drifts.
#[tokio::test]
#[ignore = "requires an LLM API key (ANTHROPIC_API_KEY or OPENAI_API_KEY)"]
async fn off_mode_runs_through_verify_only() {
    let Some(model) = boot().await else {
        eprintln!("no API key registered; skipping");
        return;
    };

    let mut agent = build_agent_with_mode(model, ReflectMode::Off)
        .await
        .expect("build agent");
    let (outputs, report) = run_with_verify(&mut agent, build_query(), &VerifyConfig::default())
        .await
        .expect("run_with_verify");

    assert!(
        outputs.iter().any(|o| o.message.role == Role::Assistant),
        "no assistant output produced"
    );
    eprintln!("[off] verify report:\n{}", report.format());
}

/// Self mode adds the `verify` tool + a system-prompt instruction telling
/// the LLM to call it before emitting the final answer. We don't assert
/// the LLM actually called it — that's self-discipline driven and
/// notoriously flaky — but we do confirm the agent ran end-to-end with
/// the augmented tool set.
#[tokio::test]
#[ignore = "requires an LLM API key (ANTHROPIC_API_KEY or OPENAI_API_KEY)"]
async fn self_mode_runs_through_with_verify_tool() {
    let Some(model) = boot().await else {
        eprintln!("no API key registered; skipping");
        return;
    };

    let mut agent = build_agent_with_mode(model, ReflectMode::Self_)
        .await
        .expect("build agent");
    let (outputs, report) = run_with_verify(&mut agent, build_query(), &VerifyConfig::default())
        .await
        .expect("run_with_verify");

    assert!(
        outputs.iter().any(|o| o.message.role == Role::Assistant),
        "no assistant output produced"
    );
    let verify_tool_calls = outputs
        .iter()
        .filter_map(|o| o.message.tool_calls.as_ref())
        .flatten()
        .filter(|p| {
            p.as_function()
                .map(|(_, name, _)| name == "verify")
                .unwrap_or(false)
        })
        .count();
    eprintln!(
        "[self] verify tool calls observed: {verify_tool_calls}; verify report:\n{}",
        report.format()
    );
}

/// Forced mode wraps the agent: after the turn finishes we unconditionally
/// run a reflect call on the draft answer and follow its verdict. We
/// confirm the wrapper produced at least one verdict and respected the
/// retry budget (≤ 1).
#[tokio::test]
#[ignore = "requires an LLM API key (ANTHROPIC_API_KEY or OPENAI_API_KEY)"]
async fn forced_mode_runs_through_with_reflect_verdicts() {
    let Some(model) = boot().await else {
        eprintln!("no API key registered; skipping");
        return;
    };

    let provider: AgentProvider = default_provider().await.clone();
    let mut agent = build_agent_with_mode(model, ReflectMode::Forced)
        .await
        .expect("build agent");
    let outcome = run_with_forced_reflect(
        &mut agent,
        build_query(),
        &VerifyConfig::default(),
        &provider,
        DEFAULT_REFLECT_MODEL,
    )
    .await
    .expect("run_with_forced_reflect");

    assert!(
        outcome
            .outputs
            .iter()
            .any(|o| o.message.role == Role::Assistant),
        "no assistant output produced"
    );
    assert!(
        !outcome.reflect_verdicts.is_empty(),
        "forced mode should emit at least one reflect verdict"
    );
    assert!(
        outcome.retry_count <= 1,
        "retry budget exceeded: {}",
        outcome.retry_count
    );
    eprintln!(
        "[forced] retries: {}; verdicts: {} ({:?}); verify report:\n{}",
        outcome.retry_count,
        outcome.reflect_verdicts.len(),
        outcome
            .reflect_verdicts
            .iter()
            .map(|v| if v.is_retry() { "retry" } else { "stop" })
            .collect::<Vec<_>>(),
        outcome.verify_report.format()
    );
}

/// Hybrid mode runs the deterministic verifier per turn, hands its
/// findings into the reflect call, and may promote a low-confidence
/// `Stop` into a retry. We verify shape: at least one verdict, a per-turn
/// report for each agent turn, and the retry budget is respected. Content
/// is left to manual inspection — LLM behaviour is non-deterministic.
#[tokio::test]
#[ignore = "requires an LLM API key (ANTHROPIC_API_KEY or OPENAI_API_KEY)"]
async fn hybrid_mode_runs_through_with_per_turn_verify() {
    let Some(model) = boot().await else {
        eprintln!("no API key registered; skipping");
        return;
    };

    let provider: AgentProvider = default_provider().await.clone();
    let mut agent = build_agent_with_mode(model, ReflectMode::Hybrid)
        .await
        .expect("build agent");
    let outcome = run_with_hybrid(
        &mut agent,
        build_query(),
        &VerifyConfig::default(),
        &provider,
        DEFAULT_REFLECT_MODEL,
    )
    .await
    .expect("run_with_hybrid");

    assert!(
        outcome
            .outputs
            .iter()
            .any(|o| o.message.role == Role::Assistant),
        "no assistant output produced"
    );
    assert!(
        !outcome.reflect_verdicts.is_empty(),
        "hybrid mode should emit at least one reflect verdict"
    );
    // One verify report per agent turn (initial + each retry).
    assert_eq!(
        outcome.per_turn_verify_reports.len(),
        outcome.retry_count + 1,
        "per-turn report count must equal retry_count + 1"
    );
    assert!(
        outcome.retry_count <= 1,
        "retry budget exceeded: {}",
        outcome.retry_count
    );
    eprintln!(
        "[hybrid] retries: {} (low-confidence promotions: {}); verdicts: {} ({:?}); final verify report:\n{}",
        outcome.retry_count,
        outcome.low_confidence_promotions,
        outcome.reflect_verdicts.len(),
        outcome
            .reflect_verdicts
            .iter()
            .map(|v| if v.is_retry() { "retry" } else { "stop" })
            .collect::<Vec<_>>(),
        outcome.verify_report.format()
    );
}
