//! ```sh
//! # Interactive REPL
//! cargo run
//!
//! # Initialize a dataset preset then start the REPL
//! cargo run -- --preset finance-bench
//!
//! # With a custom store dir or model
//! cargo run -- --store-dir ~/.my-store --model anthropic/claude-haiku-4-5-20251001 --preset finance-bench
//! ```

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::RwLock;

use ailoy::{
    agent::{Agent, AgentProvider},
    lang_model::LangModelProvider,
    message::{Message, Part, Role},
};
use anyhow::Result;
use clap::Parser;
use futures::StreamExt;
use rustyline::{DefaultEditor, error::ReadlineError};
use speedwagon::{FileType, SharedStore, SpeedwagonSpec, Store, build_tools};

use speedwagon::preset::{PresetKind, setup_docset};

#[derive(Parser)]
#[command(name = "speedwagon", about = "Interactive document Q&A agent")]
struct Cli {
    /// Directory for the document store (index + corpus files)
    #[arg(long, default_value = "~/.speedwagon")]
    store_dir: String,

    /// Language model (e.g. openai/gpt-4o-mini, anthropic/claude-haiku-4-5-20251001)
    #[arg(long, default_value = "openai/gpt-4o-mini")]
    model: String,

    /// Initialize the store from a predefined dataset before starting
    #[arg(long)]
    preset: Option<PresetKind>,
}

fn resolve_dir(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(path))
    } else {
        PathBuf::from(path)
    }
}

async fn build_agent(store_dir: &Path, model: &str, provider: &AgentProvider) -> Result<Agent> {
    let store: SharedStore = Arc::new(RwLock::new(Store::new(store_dir)?));

    let spec = SpeedwagonSpec::new().model(model).into_spec();
    let mut provider = provider.clone();
    provider.tools = build_tools(store);
    Agent::try_with_provider(spec, &provider).await
}

async fn run_query(agent: &mut Agent, input: &str) -> Result<()> {
    let query = Message::new(Role::User).with_contents([Part::text(input)]);
    let mut stream = agent.run(query);

    while let Some(output) = stream.next().await {
        let output = output?;
        let msg = &output.message;

        match msg.role {
            Role::Assistant => {
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
            _ => {}
        }
    }

    println!();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if Path::new(".env").exists() {
        dotenvy::dotenv()?;
    }

    let store_dir = {
        let base = resolve_dir(&cli.store_dir);
        match &cli.preset {
            Some(preset) => base.join(preset.to_string()),
            None => base,
        }
    };

    if let Some(ref preset) = cli.preset {
        let mut store = Store::new(&store_dir)?;
        setup_docset(&mut store, preset).await?;
    }

    let mut provider = AgentProvider::new();
    let mut model_provider = LangModelProvider::new();
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        model_provider.insert("openai/*".into(), LangModelProvider::openai(key));
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        model_provider.insert("anthropic/*".into(), LangModelProvider::anthropic(key));
    }
    if let Ok(key) = std::env::var("GEMINI_API_KEY") {
        model_provider.insert("google/*".into(), LangModelProvider::gemini(key));
    }
    provider.models = model_provider;

    let mut agent = build_agent(&store_dir, &cli.model, &provider).await?;
    let doc_count = Store::new(&store_dir)?.count();

    println!();
    println!(
        "  Speedwagon  |  model: {}  |  docs: {}",
        cli.model, doc_count
    );
    println!("  Commands: /list  /ingest <path>  /purge <id>  /clear  /exit");
    println!("  {}", "─".repeat(60));

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

        if input == "/exit" {
            break;
        } else if input == "/clear" {
            agent = build_agent(&store_dir, &cli.model, &provider).await?;
            println!("Conversation cleared.");
        } else if input == "/list" {
            let store = Store::new(&store_dir)?;
            let docs = store.list(false, 0, u32::MAX)?;
            if docs.is_empty() {
                println!("No documents indexed.");
            } else {
                println!();
                for doc in &docs {
                    let short_id = &doc.id[..8.min(doc.id.len())];
                    println!("  {short_id}  {}  ({} bytes)", doc.title, doc.len);
                }
                println!("\n  {} document(s)", docs.len());
            }
        } else if let Some(path_str) = input.strip_prefix("/ingest ") {
            let path = Path::new(path_str.trim());
            if !path.exists() {
                eprintln!("File not found: {}", path.display());
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let filetype = match ext.to_lowercase().as_str() {
                "pdf" => FileType::PDF,
                _ => {
                    eprintln!("Unsupported file type '.{ext}' — only PDF is supported");
                    continue;
                }
            };
            println!("Ingesting {}...", path.display());
            let bytes = std::fs::read(path)?;
            let mut write_store = Store::new(&store_dir)?;
            let id = write_store.ingest(bytes, filetype).await?;
            drop(write_store);
            agent = build_agent(&store_dir, &cli.model, &provider).await?;
            println!("Ingested (id: {id})  —  agent rebuilt.");
        } else if let Some(id_str) = input.strip_prefix("/purge ") {
            let id_str = id_str.trim();
            let id: uuid::Uuid = if let Ok(id) = id_str.parse() {
                id
            } else {
                let store = Store::new(&store_dir)?;
                let docs = store.list(false, 0, u32::MAX)?;
                let matches: Vec<_> = docs.iter().filter(|d| d.id.starts_with(id_str)).collect();
                match matches.len() {
                    0 => {
                        eprintln!("No document with id prefix '{id_str}'");
                        continue;
                    }
                    1 => matches[0].id.parse().unwrap(),
                    _ => {
                        eprintln!("Ambiguous prefix '{id_str}' — {} matches", matches.len());
                        continue;
                    }
                }
            };
            let mut write_store = Store::new(&store_dir)?;
            match write_store.purge(id)? {
                Some(doc) => {
                    drop(write_store);
                    agent = build_agent(&store_dir, &cli.model, &provider).await?;
                    println!("Purged '{}' — agent rebuilt.", doc.title);
                }
                None => eprintln!("Document not found: {id}"),
            }
        } else {
            println!();
            if let Err(e) = run_query(&mut agent, &input).await {
                eprintln!("ERROR: {e}");
            }
        }
    }

    println!("\nGoodbye!");
    Ok(())
}
