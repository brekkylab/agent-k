//! End-to-end shape tests for `ReflectMode::{Off, Forced}` on the demo
//! task (unstructured experiment log → structured `(timestamp, metric,
//! value)` parse).
//!
//! Both tests are `#[ignore]`d because they require a live LLM API key
//! (`ANTHROPIC_API_KEY` preferred, `OPENAI_API_KEY` accepted). They make
//! no LLM-dependent assertions — the agent's exact output varies
//! run-to-run — but they do verify that each mode's pipeline runs to
//! completion and that the verify report / reflect verdicts surface in
//! the expected shape.
//!
//! Run them:
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
    DEFAULT_ESCALATE_MODEL, DEFAULT_REFLECT_MODEL, ReflectMode, VerifyConfig,
    build_agent_with_mode, register_provider_from_env, run_with_forced_reflect, run_with_verify,
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

/// Forced mode wraps the agent: after each turn we run a Haiku reflect
/// call on the draft answer and follow its verdict, escalating to the
/// stronger model when the first-pass confidence is low. We confirm the
/// wrapper produced at least one verdict and respected the retry budget
/// (≤ 1) and that the escalation count is consistent with the verdict
/// chain length.
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
        DEFAULT_ESCALATE_MODEL,
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
    // The verdict chain holds one first-pass verdict per turn plus one
    // additional verdict per escalation. Lower bound: turns ≤ verdicts.
    assert!(
        outcome.reflect_verdicts.len() >= outcome.retry_count + 1,
        "verdict chain shorter than expected"
    );
    eprintln!(
        "[forced] retries: {} escalations: {}; verdicts: {} ({:?}); verify report:\n{}",
        outcome.retry_count,
        outcome.escalations,
        outcome.reflect_verdicts.len(),
        outcome
            .reflect_verdicts
            .iter()
            .map(|v| if v.is_retry() { "retry" } else { "stop" })
            .collect::<Vec<_>>(),
        outcome.verify_report.format()
    );
}
