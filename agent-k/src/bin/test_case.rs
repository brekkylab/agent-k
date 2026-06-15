//! Run a single test case against the chosen agent, then drop into
//! interactive mode.
//!
//! cargo run -p agent-k --bin test_case -- coworker 0
//! cargo run -p agent-k --bin test_case -- coworker 0 --model claude
//! cargo run -p agent-k --bin test_case -- coworker 0 --model gemini
//! cargo run -p agent-k --bin test_case -- coworker 0 --model kimi
//! cargo run -p agent-k --bin test_case -- deep-research 0 --model claude

use std::io::{self, BufRead, IsTerminal, Write};

use std::sync::Arc;

use agent_k::agents::{
    CoworkerSandboxOptions, get_coworker_agent_with_opts, get_deep_research_agent,
    get_speedwagon_agent,
};
use agent_k::knowledge_base::{FileType, PdfEngine, SharedStore, Store};
use ailoy::{
    agent::Agent,
    lang_model::LangModelAPISchema,
    message::{Message, Part, Role},
};
use futures::StreamExt;
use tokio::sync::RwLock;
use url::Url;

#[path = "test_case/cases/mod.rs"]
mod cases;
use cases::{Case, get_coworker_cases, get_deep_research_cases, get_speedwagon_cases};

const COWORKER_AGENT_NAME: &str = "minerva";
const DEEP_RESEARCH_AGENT_NAME: &str = "vegapunk";
const SPEEDWAGON_AGENT_NAME: &str = "jonathan";
const OPENAI_MODEL: &str = "openai/gpt-5.5";
const CLAUDE_MODEL: &str = "anthropic/claude-opus-4-7";
const GEMINI_MODEL: &str = "google/gemini-3.5-flash";
const KIMI_MODEL: &str = "moonshot/kimi-k2.6";
const ARTIFACT_DIR: &str = "./test/artifacts";
const DATA_DIR: &str = "./test/data";
const SHARED_DATA_DIR: &str = "./test/shared_data";
const CORPUS_DIR: &str = "./test/corpus";

enum AgentKind {
    Coworker,
    DeepResearch,
    Speedwagon,
}

impl AgentKind {
    fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "coworker" => Ok(Self::Coworker),
            "deep-research" | "deep_research" => Ok(Self::DeepResearch),
            "speedwagon" => Ok(Self::Speedwagon),
            other => anyhow::bail!(
                "invalid agent '{}', expected 'coworker', 'deep-research', or 'speedwagon'",
                other
            ),
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Coworker => COWORKER_AGENT_NAME,
            Self::DeepResearch => DEEP_RESEARCH_AGENT_NAME,
            Self::Speedwagon => SPEEDWAGON_AGENT_NAME,
        }
    }
    fn log_prefix(&self) -> &'static str {
        match self {
            Self::Coworker => "coworker",
            Self::DeepResearch => "deep-research",
            Self::Speedwagon => "speedwagon",
        }
    }
}

enum InputSource {
    Stdin,
    Tty(io::BufReader<std::fs::File>),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    if let Ok(key) = std::env::var("KIMI_API_KEY") {
        let mut provider = ailoy::agent::default_provider_mut();
        provider.models.insert_api(
            "moonshot/kimi-*".into(),
            LangModelAPISchema::ChatCompletion,
            Url::parse("https://api.moonshot.ai/v1/chat/completions")?,
            Some(key),
        );
    }

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut positional: Vec<&str> = Vec::new();
    let mut model_arg: Option<&str> = None;
    let mut no_skill = false;
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "--model" | "-m" => {
                let v = argv.get(i + 1).ok_or_else(|| {
                    anyhow::anyhow!("--model requires a value (openai|claude|gemini|kimi)")
                })?;
                model_arg = Some(v.as_str());
                i += 2;
            }
            s if s.starts_with("--model=") => {
                model_arg = Some(&s["--model=".len()..]);
                i += 1;
            }
            "--no-skill" => {
                no_skill = true;
                i += 1;
            }
            s => {
                positional.push(s);
                i += 1;
            }
        }
    }

    if positional.len() != 2 {
        eprintln!(
            "usage: test_case <agent> <case_no> [--model openai|claude|gemini|kimi] [--no-skill]\n\
             agents: coworker, deep-research, speedwagon"
        );
        std::process::exit(2);
    }
    let agent_kind = AgentKind::parse(positional[0])?;
    let case_no: usize = positional[1].parse().map_err(|_| {
        anyhow::anyhow!(
            "invalid case number '{}', expected a non-negative integer",
            positional[1]
        )
    })?;

    let agent_model = match model_arg {
        None | Some("openai") => OPENAI_MODEL,
        Some("claude") => CLAUDE_MODEL,
        Some("gemini") => GEMINI_MODEL,
        Some("kimi") => KIMI_MODEL,
        Some(other) => anyhow::bail!(
            "invalid --model '{}', expected 'openai', 'claude', 'gemini', or 'kimi'",
            other
        ),
    };

    let mut cases = match agent_kind {
        AgentKind::Coworker => get_coworker_cases(),
        AgentKind::DeepResearch => get_deep_research_cases(),
        AgentKind::Speedwagon => get_speedwagon_cases(),
    };
    if case_no >= cases.len() {
        anyhow::bail!(
            "case {} out of range (have {} {} case(s))",
            case_no,
            cases.len(),
            agent_kind.log_prefix()
        );
    }
    let case = cases.swap_remove(case_no);

    prepare_dir(ARTIFACT_DIR);
    prepare_dir(DATA_DIR);
    prepare_dir(SHARED_DATA_DIR);
    prepare_dir(CORPUS_DIR);
    write_case_files(&case)?;

    // Build the Speedwagon corpus store when the case ships documents. Coworker
    // and DeepResearch only get a store (and thus the `subagent_speedwagon`
    // sub-agent) when one is present; Speedwagon always needs one.
    let corpus_store: Option<SharedStore> = if case.corpus_files.is_empty() {
        None
    } else {
        Some(build_corpus_store(&case.corpus_files).await?)
    };
    // For delegation, run the Speedwagon sub-agent on the corpus-recommended
    // model of the parent's provider (not the parent's own, which may be a
    // model that fares poorly in the corpus loop, e.g. gemini-3.5-flash).
    let corpus_model: Option<String> = corpus_store
        .as_ref()
        .map(|_| speedwagon_model_for(agent_model).to_string());

    let mut agent = match agent_kind {
        AgentKind::Coworker => {
            let opts = CoworkerSandboxOptions {
                sandbox_name: None,
                persist: false,
                with_skill: !no_skill,
                corpus_store: corpus_store.clone(),
                corpus_model: corpus_model.clone(),
            };
            get_coworker_agent_with_opts(
                agent_kind.name(),
                agent_model,
                DATA_DIR,
                SHARED_DATA_DIR,
                ARTIFACT_DIR,
                opts,
            )
            .await?
        }
        AgentKind::DeepResearch => {
            get_deep_research_agent(
                agent_kind.name(),
                agent_model,
                ARTIFACT_DIR,
                corpus_store.clone(),
                corpus_model.clone(),
            )
            .await?
        }
        AgentKind::Speedwagon => {
            let store = corpus_store.clone().ok_or_else(|| {
                anyhow::anyhow!("speedwagon case must define corpus_files")
            })?;
            // Speedwagon is corpus-QA only; run it on the corpus-recommended
            // (lightweight) model for the chosen provider, not the heavier shared
            // default (e.g. gemini-3.5-flash, which is slow in the corpus loop).
            let sw_model = speedwagon_model_for(agent_model);
            get_speedwagon_agent(agent_kind.name(), sw_model, store, true).await?
        }
    };
    println!(
        "[{}] starting as '{}' ({}) — case #{}",
        agent_kind.log_prefix(),
        agent_kind.name(),
        agent_model,
        case_no
    );

    if let Err(e) = stream_turn(&mut agent, case.query, agent_kind.log_prefix()).await {
        println!("[error] {e}");
    }

    let stdin_is_tty = io::stdin().is_terminal();
    let source = if stdin_is_tty {
        InputSource::Stdin
    } else {
        match std::fs::File::open("/dev/tty") {
            Ok(f) => InputSource::Tty(io::BufReader::new(f)),
            Err(_) => return Ok(()),
        }
    };

    let (req_tx, mut req_rx) = tokio::sync::mpsc::channel::<()>(1);
    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<Option<String>>(1);
    std::thread::spawn(move || {
        let mut source = source;
        while req_rx.blocking_recv().is_some() {
            eprint!("> ");
            io::stderr().flush().ok();
            let mut buf = String::new();
            let payload = match &mut source {
                InputSource::Stdin => io::stdin().read_line(&mut buf),
                InputSource::Tty(r) => r.read_line(&mut buf),
            };
            let payload = match payload {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(buf),
            };
            let done = payload.is_none();
            if line_tx.blocking_send(payload).is_err() || done {
                break;
            }
        }
    });

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        if req_tx.send(()).await.is_err() {
            break;
        }
        tokio::select! {
            _ = &mut ctrl_c => {
                println!();
                break;
            }
            msg = line_rx.recv() => {
                match msg.flatten() {
                    None => {
                        println!();
                        break;
                    }
                    Some(line) => {
                        let input = line.trim().to_string();
                        if !input.is_empty() {
                            let query = Message::new(Role::User).with_contents([Part::text(&input)]);
                            if let Err(e) = stream_turn(&mut agent, query, agent_kind.log_prefix()).await {
                                println!("[error] {e}");
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn prepare_dir(dir: &str) {
    let path = std::path::Path::new(dir);
    if path.exists() {
        if let Err(e) = std::fs::remove_dir_all(path) {
            println!("[warn] failed to clean {}: {e}", path.display());
        }
    }
    if let Err(e) = std::fs::create_dir_all(path) {
        println!("[warn] failed to create {}: {e}", path.display());
    }
}

fn write_case_files(case: &Case) -> anyhow::Result<()> {
    write_files(DATA_DIR, &case.files)?;
    write_files(SHARED_DATA_DIR, &case.shared_files)?;
    Ok(())
}

fn write_files(dir: &str, files: &[(Vec<u8>, std::path::PathBuf)]) -> anyhow::Result<()> {
    let base = std::path::Path::new(dir);
    for (bytes, rel) in files {
        let dst = base.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dst, bytes)?;
        println!("[case] wrote {}", dst.display());
    }
    Ok(())
}

/// The Speedwagon sub-agent model for a parent model's provider, mirroring the
/// backend's `speedwagon_model_for_parent` (which the backend derives from
/// `AgentType::Speedwagon.chain()`). Keeps a delegated corpus question off a
/// model that is poor for the corpus loop while staying on the same provider.
fn speedwagon_model_for(parent_model: &str) -> &'static str {
    match parent_model.split('/').next().unwrap_or("") {
        "openai" => "openai/gpt-5.4-mini",
        "anthropic" => "anthropic/claude-sonnet-4-6",
        "google" => "google/gemini-3.1-flash-lite",
        "moonshot" | "moonshotai" => "moonshotai/kimi-k2.6",
        _ => "openai/gpt-5.4-mini",
    }
}

/// Map a corpus file's extension to a `FileType`, matching the backend's
/// `indexable_filetype` (so `.txt`/`.markdown` index as Markdown, unlike
/// `FileType::from_path` which only knows pdf/md/html).
fn corpus_filetype(path: &std::path::Path) -> Option<FileType> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Some(FileType::PDF),
        "md" | "markdown" | "txt" => Some(FileType::MD),
        "html" | "htm" => Some(FileType::HTML),
        _ => None,
    }
}

/// Build a Speedwagon corpus store from the case's `corpus_files`, indexing each
/// into a fresh store under `CORPUS_DIR`. The file's extension picks its
/// `FileType`; unsupported files are skipped. Mirrors what the backend's
/// knowledge resync does, minus the per-project plumbing.
async fn build_corpus_store(
    corpus_files: &[(Vec<u8>, std::path::PathBuf)],
) -> anyhow::Result<SharedStore> {
    let mut store = Store::new(format!("{CORPUS_DIR}/.speedwagon"))?;
    let items: Vec<(Vec<u8>, FileType)> = corpus_files
        .iter()
        .filter_map(|(bytes, path)| corpus_filetype(path).map(|ft| (bytes.clone(), ft)))
        .collect();
    let result = store.ingest_many(items, PdfEngine::default()).await?;
    println!(
        "[corpus] indexed {} document(s), {} failed",
        result.succeeded.len(),
        result.failed.len()
    );
    for f in &result.failed {
        println!("[corpus] failed to index item {}: {}", f.index, f.error);
    }
    Ok(Arc::new(RwLock::new(store)))
}

async fn stream_turn(agent: &mut Agent, query: Message, log_prefix: &str) -> anyhow::Result<()> {
    let mut stream = agent.run(query);
    while let Some(event) = stream.next().await {
        let event = event?;
        let msg = &event.message;
        match msg.role {
            Role::Assistant => {
                for part in &msg.contents {
                    if let Some(t) = part.as_text() {
                        if !t.is_empty() {
                            println!("{t}");
                            io::stdout().flush().ok();
                        }
                    }
                }
                if let Some(tcs) = &msg.tool_calls {
                    for tc in tcs {
                        if let Some((_id, name, args)) = tc.as_function() {
                            let args_json = serde_json::to_string(args)
                                .unwrap_or_else(|_| "<unprintable>".into());
                            println!("[{log_prefix}] tool: {name} {args_json}");
                        }
                    }
                }
            }
            Role::Tool => {
                for part in &msg.contents {
                    if let Some(t) = part.as_text() {
                        println!("[{log_prefix}] tool result: {t}");
                    } else if let Some(v) = part.as_value() {
                        let s = serde_json::to_string(v).unwrap_or_else(|_| "<unprintable>".into());
                        println!("[{log_prefix}] tool result: {s}");
                    }
                }
            }
            _ => {}
        }
    }
    println!();
    Ok(())
}
