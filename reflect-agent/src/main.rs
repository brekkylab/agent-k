//! ```sh
//! # Interactive REPL with default model
//! cargo run -p reflect-agent
//!
//! # Single query (non-interactive), useful for scripts and tests
//! cargo run -p reflect-agent -- --query "What is 2 + 2?"
//!
//! # Override model
//! cargo run -p reflect-agent -- --model openai/gpt-4o-mini
//! ```

use ailoy::{
    agent::default_provider_mut,
    message::{Message, Part, Role},
};
use anyhow::Result;
use clap::Parser;
use reflect_agent::{
    DEFAULT_MODEL, VerifyConfig, build_agent, register_provider_from_env, run_with_verify,
};
use rustyline::{DefaultEditor, error::ReadlineError};

#[derive(Parser)]
#[command(
    name = "reflect-agent",
    about = "Single lead agent with bash + python + web_search tools and a post-hoc verify gate"
)]
struct Cli {
    /// Language model id, e.g. `openai/gpt-4o-mini`,
    /// `anthropic/claude-haiku-4-5-20251001`, `google/gemini-2.5-flash`.
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,

    /// Run a single query non-interactively and exit. Useful for smoke tests
    /// and scripted workflows.
    #[arg(long)]
    query: Option<String>,
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

    // Populate ailoy's process-global provider once at boot.
    register_provider_from_env(&mut *default_provider_mut().await);

    let mut agent = build_agent(&cli.model).await?;

    if let Some(q) = cli.query {
        run_query(&mut agent, &q).await?;
        return Ok(());
    }

    println!();
    println!("  reflect-agent  |  model: {}", cli.model);
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
        if let Err(e) = run_query(&mut agent, &line).await {
            eprintln!("ERROR: {e}");
        }
    }

    println!("\nGoodbye!");
    Ok(())
}

/// Stream one user turn, print assistant text + tool-call markers, and
/// emit the verify report to stderr only when at least one issue was
/// flagged (otherwise stderr stays clean).
async fn run_query(agent: &mut ailoy::agent::Agent, input: &str) -> Result<()> {
    let query = Message::new(Role::User).with_contents([Part::text(input)]);
    let config = VerifyConfig::default();
    let (outputs, report) = run_with_verify(agent, query, &config).await?;

    for output in &outputs {
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

    if !report.is_empty() {
        eprintln!("\n─── verify gate findings ───");
        eprint!("{}", report.format());
    }
    Ok(())
}
