//! ```sh
//! # Interactive REPL with default model
//! cargo run -p reflect-agent
//!
//! # Single query (non-interactive), useful for smoke tests / scripts
//! cargo run -p reflect-agent -- --query "What is 2 + 2?"
//!
//! # Override model
//! cargo run -p reflect-agent -- --model openai/gpt-4o-mini
//!
//! # Choose reflect mode (off | self | forced)
//! cargo run -p reflect-agent -- --reflect-mode self  --query "..."
//! cargo run -p reflect-agent -- --reflect-mode forced --query "..."
//!
//! # Show both verify and reflect outputs (regardless of mode)
//! cargo run -p reflect-agent -- --reflect-mode forced --verbose --query "..."
//! ```

use ailoy::{
    agent::{Agent, AgentProvider, default_provider, default_provider_mut},
    message::{Message, Part, Role},
};
use anyhow::Result;
use clap::Parser;
use reflect_agent::{
    DEFAULT_MODEL, DEFAULT_REFLECT_MODEL, ReflectMode, VerifyConfig, VerifyReport,
    build_agent_with_mode, register_provider_from_env, run_with_forced_reflect, run_with_hybrid,
    run_with_verify,
};
use rustyline::{DefaultEditor, error::ReadlineError};

#[derive(Parser)]
#[command(
    name = "reflect-agent",
    about = "Single lead agent with bash + python + web_search tools, plus optional verify/reflect gates"
)]
struct Cli {
    /// Language model id, e.g. `openai/gpt-4o-mini`,
    /// `anthropic/claude-haiku-4-5-20251001`, `google/gemini-2.5-flash`.
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,

    /// Run a single query non-interactively and exit.
    #[arg(long)]
    query: Option<String>,

    /// Reflect strategy: `off` (default), `self` (verify tool + system
    /// prompt), or `forced` (wrapper-driven retry budget = 1).
    #[arg(long, default_value = "off")]
    reflect_mode: String,

    /// Model used for the reflect call in `self` and `forced` modes.
    #[arg(long, default_value = DEFAULT_REFLECT_MODEL)]
    reflect_model: String,

    /// Always emit the verify report to stderr, even when reflect mode is
    /// `self` or `forced`. Useful for comparing what each layer flags.
    #[arg(long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let mode = ReflectMode::parse(&cli.reflect_mode)?;

    register_provider_from_env(&mut *default_provider_mut().await);

    let mut agent = build_agent_with_mode(&cli.model, mode).await?;

    if let Some(q) = cli.query {
        run_query(&mut agent, &q, mode, &cli.reflect_model, cli.verbose).await?;
        return Ok(());
    }

    println!();
    println!(
        "  reflect-agent  |  model: {}  |  reflect: {}",
        cli.model,
        mode.as_str()
    );
    println!("  Commands: /exit");
    println!("  {}", "─".repeat(60));

    let mut rl = DefaultEditor::new()?;
    loop {
        let line = match rl.readline("\n> ") {
            Ok(l) => l.trim().to_string(),
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        if line.is_empty() {
            continue;
        }
        rl.add_history_entry(&line)?;
        if line == "/exit" {
            break;
        }
        if let Err(e) = run_query(&mut agent, &line, mode, &cli.reflect_model, cli.verbose).await {
            eprintln!("ERROR: {e}");
        }
    }

    println!("\nGoodbye!");
    Ok(())
}

/// Stream one user turn through the chosen reflect mode and print:
///
/// - assistant text + tool-call markers on stdout (every mode);
/// - verify report on stderr — always in `off`, only with `--verbose` in
///   the LLM-driven modes;
/// - reflect verdicts on stderr — only when `forced` mode actually emits
///   them (`self` mode's verdicts surface as tool results in the stdout
///   stream, so they don't need a separate channel).
async fn run_query(
    agent: &mut Agent,
    input: &str,
    mode: ReflectMode,
    reflect_model: &str,
    verbose: bool,
) -> Result<()> {
    let query = Message::new(Role::User).with_contents([Part::text(input)]);
    let verify_config = VerifyConfig::default();

    match mode {
        ReflectMode::Off | ReflectMode::Self_ => {
            // Both modes use the same outer driver. The difference is at
            // build_agent_with_mode time (Self_ added the verify tool +
            // system prompt). At run time we just stream and verify.
            let (outputs, report) = run_with_verify(agent, query, &verify_config).await?;
            print_assistant_stream(&outputs);
            // Off: always show the verify report. Self_: only with --verbose.
            let show_verify = matches!(mode, ReflectMode::Off) || verbose;
            if show_verify {
                print_verify_report(&report);
            }
        }
        ReflectMode::Forced => {
            let provider: AgentProvider = default_provider().await.clone();
            let outcome =
                run_with_forced_reflect(agent, query, &verify_config, &provider, reflect_model)
                    .await?;
            print_assistant_stream(&outcome.outputs);
            // Reflect verdicts always print in forced mode (that's the point).
            print_reflect_verdicts(&outcome.reflect_verdicts, outcome.retry_count);
            // Verify report only with --verbose to keep the comparison clean.
            if verbose {
                print_verify_report(&outcome.verify_report);
            }
        }
        ReflectMode::Hybrid => {
            let provider: AgentProvider = default_provider().await.clone();
            let outcome =
                run_with_hybrid(agent, query, &verify_config, &provider, reflect_model).await?;
            print_assistant_stream(&outcome.outputs);
            // Hybrid is the comparison mode, so its outputs always include
            // both verdicts *and* the verifier's per-turn findings — that's
            // the entire point of using this mode.
            print_reflect_verdicts(&outcome.reflect_verdicts, outcome.retry_count);
            if outcome.low_confidence_promotions > 0 {
                eprintln!(
                    "       (promoted {} low-confidence stop(s) into retries)",
                    outcome.low_confidence_promotions
                );
            }
            print_verify_report(&outcome.verify_report);
        }
    }

    Ok(())
}

fn print_assistant_stream(outputs: &[ailoy::message::MessageOutput]) {
    for output in outputs {
        let msg = &output.message;
        if msg.role != Role::Assistant {
            continue;
        }
        for part in &msg.contents {
            if let Some(text) = part.as_text() {
                if !text.is_empty() {
                    print!("{text}");
                }
            }
        }
        if let Some(ref tool_calls) = msg.tool_calls {
            for part in tool_calls {
                if let Some((_id, name, _args)) = part.as_function() {
                    println!("\n  [→ {name}]");
                }
            }
        }
    }
    println!();
}

fn print_verify_report(report: &VerifyReport) {
    if report.is_empty() {
        return;
    }
    eprintln!("\n─── verify gate findings ───");
    eprint!("{}", report.format());
}

fn print_reflect_verdicts(verdicts: &[reflect_agent::ReflectVerdict], retry_count: usize) {
    if verdicts.is_empty() {
        return;
    }
    eprintln!("\n─── reflect gate verdicts (retries: {}) ───", retry_count);
    for (i, v) in verdicts.iter().enumerate() {
        let conf = match v.confidence() {
            Some(c) => format!(" (conf={c:.2})"),
            None => String::new(),
        };
        match v {
            reflect_agent::ReflectVerdict::Stop { rationale, .. } => {
                eprintln!("- [{i}] stop{conf}: {rationale}");
            }
            reflect_agent::ReflectVerdict::Retry {
                rationale,
                next_query,
                ..
            } => {
                eprintln!("- [{i}] retry{conf}: {rationale}");
                eprintln!("       next_query: {next_query}");
            }
        }
    }
}
