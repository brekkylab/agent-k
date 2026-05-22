//! Run a single test case against the chosen agent, then drop into
//! interactive mode.
//!
//! cargo run -p agent-k --bin test_case -- coworker 0
//! cargo run -p agent-k --bin test_case -- coworker 0 --model claude
//! cargo run -p agent-k --bin test_case -- coworker 0 --model gemini
//! cargo run -p agent-k --bin test_case -- coworker 0 --model kimi
//! cargo run -p agent-k --bin test_case -- deep-research 0 --model claude

use std::io::{self, BufRead, IsTerminal, Write};

use agent_k::agents::{get_coworker_agent, get_deep_research_agent};
use ailoy::{
    agent::Agent,
    lang_model::LangModelAPISchema,
    message::{Message, Part, Role},
};
use futures::StreamExt;
use url::Url;

#[path = "test_case/cases/mod.rs"]
mod cases;
use cases::{Case, get_coworker_cases, get_deep_research_cases};

const COWORKER_AGENT_NAME: &str = "minerva";
const DEEP_RESEARCH_AGENT_NAME: &str = "vegapunk";
const OPENAI_MODEL: &str = "openai/gpt-5.5";
const CLAUDE_MODEL: &str = "anthropic/claude-opus-4-7";
const GEMINI_MODEL: &str = "gemini/gemini-3.5-flash";
const KIMI_MODEL: &str = "moonshot/kimi-k2.6";
const ARTIFACT_DIR: &str = "./test/artifacts";
const DATA_DIR: &str = "./test/data";
const SHARED_DATA_DIR: &str = "./test/shared_data";

enum AgentKind {
    Coworker,
    DeepResearch,
}

impl AgentKind {
    fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "coworker" => Ok(Self::Coworker),
            "deep-research" | "deep_research" => Ok(Self::DeepResearch),
            other => anyhow::bail!(
                "invalid agent '{}', expected 'coworker' or 'deep-research'",
                other
            ),
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Coworker => COWORKER_AGENT_NAME,
            Self::DeepResearch => DEEP_RESEARCH_AGENT_NAME,
        }
    }
    fn log_prefix(&self) -> &'static str {
        match self {
            Self::Coworker => "coworker",
            Self::DeepResearch => "deep-research",
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
            s => {
                positional.push(s);
                i += 1;
            }
        }
    }

    if positional.len() != 2 {
        eprintln!(
            "usage: test_case <agent> <case_no> [--model openai|claude|gemini|kimi]\n\
             agents: coworker, deep-research"
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
    write_case_files(&case)?;

    let mut agent = match agent_kind {
        AgentKind::Coworker => {
            get_coworker_agent(
                agent_kind.name(),
                agent_model,
                DATA_DIR,
                SHARED_DATA_DIR,
                ARTIFACT_DIR,
            )
            .await?
        }
        AgentKind::DeepResearch => {
            get_deep_research_agent(agent_kind.name(), agent_model, ARTIFACT_DIR).await?
        }
    };
    println!(
        "[{}] starting as '{}' ({}) — case #{}",
        agent_kind.log_prefix(),
        agent_kind.name(),
        agent_model,
        case_no
    );

    if let Err(e) = stream_turn(&mut agent, case.query).await {
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
                            if let Err(e) = stream_turn(&mut agent, query).await {
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

async fn stream_turn(agent: &mut Agent, query: Message) -> anyhow::Result<()> {
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
                            println!("[coworker] tool: {name} {args_json}");
                        }
                    }
                }
            }
            Role::Tool => {
                for part in &msg.contents {
                    if let Some(t) = part.as_text() {
                        println!("[coworker] tool result: {t}");
                    } else if let Some(v) = part.as_value() {
                        let s = serde_json::to_string(v).unwrap_or_else(|_| "<unprintable>".into());
                        println!("[coworker] tool result: {s}");
                    }
                }
            }
            _ => {}
        }
    }
    println!();
    Ok(())
}
