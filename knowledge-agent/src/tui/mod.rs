mod app;
use std::{path::PathBuf, sync::Arc};

use ailoy::agent::AgentRuntime;
use anyhow::Result;
use rustyline::{DefaultEditor, error::ReadlineError};

pub use self::app::AppConfig;
use crate::{agent::build_agent, runner::run_with_trace, tools::SearchIndex};

pub async fn run_tui(
    config: AppConfig,
    mut agent: AgentRuntime,
    search_index: Arc<SearchIndex>,
    corpus_dirs: Vec<PathBuf>,
) -> Result<()> {
    println!();
    println!("  Knowledge Agent  |  model: {}", config.agent.model_name);
    println!("  Commands: /clear  /exit");
    println!("  {}", "─".repeat(56));

    let mut rl = DefaultEditor::new()?;

    loop {
        let readline = rl.readline("\n> ");
        let input = match readline {
            Ok(line) => line.trim().to_string(),
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };

        if input.is_empty() {
            continue;
        }

        rl.add_history_entry(&input)?;

        match input.as_str() {
            "/exit" => break,
            "/clear" => {
                match build_agent(
                    &config.agent,
                    &config.tool,
                    &search_index,
                    corpus_dirs.clone(),
                )
                .await
                {
                    Ok(new_agent) => {
                        agent = new_agent;
                        println!("Conversation cleared.");
                    }
                    Err(e) => {
                        eprintln!("Failed to reset conversation: {e}");
                    }
                }
            }
            _ => {
                println!();
                if let Err(e) = run_with_trace(&mut agent, &input).await {
                    eprintln!("ERROR: {e}");
                }
            }
        }
    }

    println!("\nGoodbye!");
    Ok(())
}
