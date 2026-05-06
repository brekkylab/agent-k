//! Integration tests for the verify gate against the demo task:
//! "convert an unstructured experiment log into a structured
//! (timestamp, metric_name, value) table and produce a time-vs-metric
//! line plot."
//!
//! Two layers of coverage:
//!
//! - **Fixture-based, deterministic.** Hand-built `Message` history that
//!   replays the failure modes the demo task tends to hit (empty greps,
//!   repeated retries, hallucinated timestamps, bash errors). No LLM
//!   call. Always runs in CI.
//!
//! - **End-to-end, ignored by default.** Spins up a real agent against
//!   the configured provider (`OPENAI_API_KEY` or `ANTHROPIC_API_KEY`),
//!   feeds it a small log fixture, and just verifies that the verify
//!   pipeline produces a well-formed report — no LLM-dependent
//!   assertions on which issues fired. Run with `cargo test
//!   --test demo_task -- --ignored`.

use ailoy::{
    message::{Message, Part, Role},
    to_value,
};
use reflect_agent::{BashFailureReason, Issue, VerifyConfig, verify_run};

// ── deterministic fixture: full demo-task failure trajectory ──────────────

/// Build the assistant message that opens a tool call.
fn assistant_call(call_id: &str, name: &str, args: ailoy::datatype::Value) -> Message {
    Message::new(Role::Assistant).with_tool_calls([Part::function(
        call_id.to_string(),
        name.to_string(),
        args,
    )])
}

/// Build the matching `Role::Tool` response for `call_id`.
fn tool_result(call_id: &str, value: ailoy::datatype::Value) -> Message {
    Message::new(Role::Tool)
        .with_contents([Part::value(value)])
        .with_id(call_id)
}

fn assistant_text(text: &str) -> Message {
    Message::new(Role::Assistant).with_contents([Part::text(text)])
}

/// One full demo-task trajectory exercising every signal at once. The
/// agent (1) grepped for a bad regex three times in a row (loop +
/// empty), (2) tried to cat a missing file (bash failure), (3) wrote a
/// final summary that cites a timestamp that never appeared in the
/// tool log (unverified citation).
fn full_failure_trajectory() -> Vec<Message> {
    vec![
        // (1) Three identical greps that all return nothing.
        assistant_call("c1", "bash", to_value!({"cmd": "grep -E 'BAD' /tmp/log.txt"})),
        tool_result(
            "c1",
            to_value!({"stdout": "", "stderr": "", "exit_code": 0, "timed_out": false}),
        ),
        assistant_call("c2", "bash", to_value!({"cmd": "grep -E 'BAD' /tmp/log.txt"})),
        tool_result(
            "c2",
            to_value!({"stdout": "", "stderr": "", "exit_code": 0, "timed_out": false}),
        ),
        assistant_call("c3", "bash", to_value!({"cmd": "grep -E 'BAD' /tmp/log.txt"})),
        tool_result(
            "c3",
            to_value!({"stdout": "", "stderr": "", "exit_code": 0, "timed_out": false}),
        ),
        // (2) Cat a missing file — non-zero exit + non-empty stderr.
        assistant_call("c4", "bash", to_value!({"cmd": "cat /tmp/nope.txt"})),
        tool_result(
            "c4",
            to_value!({
                "stdout": "",
                "stderr": "cat: /tmp/nope.txt: No such file or directory",
                "exit_code": 1,
                "timed_out": false
            }),
        ),
        // (3) Final response that fabricates a timestamp.
        assistant_text(
            "Parsed the log: the spike occurred at 2026-12-31T23:59:59 with metric=42. \
             Plot written to /tmp/out.html.",
        ),
    ]
}

#[test]
fn demo_task_full_failure_trajectory_fires_all_four_signals() {
    let history = full_failure_trajectory();
    let report = verify_run(&history, &VerifyConfig::default());

    // S1 — at least one empty result (the three greps).
    assert!(
        report
            .issues
            .iter()
            .any(|i| matches!(i, Issue::EmptyResult { tool } if tool == "bash")),
        "expected EmptyResult, got: {:?}",
        report.issues
    );

    // S2 — same (bash, args) appeared 3 times, hitting the default threshold.
    assert!(
        report
            .issues
            .iter()
            .any(|i| matches!(i, Issue::LoopDetected { count: 3, .. })),
        "expected LoopDetected count=3, got: {:?}",
        report.issues
    );

    // S3 — the cited timestamp never showed up in the tool log.
    assert!(
        report.issues.iter().any(|i| matches!(
            i,
            Issue::UnverifiedCitation { citation } if citation == "2026-12-31T23:59:59"
        )),
        "expected UnverifiedCitation for fabricated timestamp, got: {:?}",
        report.issues
    );

    // S4 — `cat /tmp/nope.txt` returned exit_code=1.
    assert!(
        report.issues.iter().any(|i| matches!(
            i,
            Issue::BashFailure {
                reason: BashFailureReason::NonZeroExit { exit_code: 1 }
            }
        )),
        "expected BashFailure NonZeroExit 1, got: {:?}",
        report.issues
    );
}

#[test]
fn demo_task_clean_run_produces_no_issues() {
    // Compact happy path: one bash that returns content, one Python REPL
    // that uses the same timestamp, and a final response that cites it.
    let history = vec![
        assistant_call(
            "c1",
            "bash",
            to_value!({"cmd": "head /tmp/log.txt"}),
        ),
        tool_result(
            "c1",
            to_value!({
                "stdout": "2024-01-15T10:30:00 metric=42\n",
                "stderr": "",
                "exit_code": 0,
                "timed_out": false
            }),
        ),
        assistant_call(
            "c2",
            "python_repl",
            to_value!({"code": "print('plot ok')"}),
        ),
        tool_result(
            "c2",
            to_value!({"output": "plot ok"}),
        ),
        assistant_text(
            "Found 2024-01-15T10:30:00 metric=42 in the log. Plot rendered successfully.",
        ),
    ];

    let report = verify_run(&history, &VerifyConfig::default());
    assert!(
        report.is_empty(),
        "expected clean run, got: {:?}",
        report.issues
    );
}

// ── end-to-end (ignored by default) ───────────────────────────────────────

/// Real-LLM smoke test. Builds an agent, asks it to parse a tiny log
/// fixture, and just confirms the verify pipeline returns a well-formed
/// report. We don't assert *which* issues fired because that depends on
/// LLM behaviour run-to-run.
#[tokio::test]
#[ignore = "requires an LLM API key (OPENAI_API_KEY or ANTHROPIC_API_KEY)"]
async fn demo_task_end_to_end_pipeline() {
    use ailoy::{
        agent::default_provider_mut,
        message::{Message, Part, Role},
    };
    use reflect_agent::{build_agent, register_provider_from_env, run_with_verify};

    dotenvy::dotenv().ok();
    register_provider_from_env(&mut *default_provider_mut().await);

    // Pick whichever provider is registered. Anthropic preferred for
    // determinism on tool calls; OpenAI is the fallback.
    let model = if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        "anthropic/claude-haiku-4-5-20251001"
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        "openai/gpt-4o-mini"
    } else {
        eprintln!("no API key registered; skipping");
        return;
    };

    // Tiny log fixture written into a temp file. Three lines, three
    // distinct timestamps, one metric per line.
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    std::fs::write(
        tmp.path(),
        "2024-01-15T10:30:00 cpu=42\n\
         2024-01-15T10:31:00 cpu=51\n\
         2024-01-15T10:32:00 cpu=47\n",
    )
    .expect("write fixture");
    let log_path = tmp.path().to_string_lossy().to_string();

    let mut agent = build_agent(model).await.expect("build agent");
    let prompt = format!(
        "Parse {log_path} into a list of (timestamp, metric, value) tuples \
         using bash and python_repl. Don't try to make a plot — just print \
         the parsed list. Cite each row's timestamp from the file."
    );
    let query = Message::new(Role::User).with_contents([Part::text(prompt)]);

    let (outputs, report) = run_with_verify(&mut agent, query, &VerifyConfig::default())
        .await
        .expect("run_with_verify");

    // Sanity: the agent produced at least one assistant message.
    assert!(
        outputs.iter().any(|o| o.message.role == Role::Assistant),
        "no assistant output produced"
    );

    // Sanity: the report rendered without panicking. We don't assert
    // on specific issues — LLM behaviour is non-deterministic — but
    // we do log them so the developer sees what fired.
    if !report.is_empty() {
        eprintln!("verify findings:\n{}", report.format());
    }
}
