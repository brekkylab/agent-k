//! Run a single test case against the coworker agent, then drop into
//! interactive mode.
//!
//! cargo run -p agent-k --bin test_case -- 0
//! cargo run -p agent-k --bin test_case -- 0 --model claude
//! cargo run -p agent-k --bin test_case -- 0 --model gemini
//! cargo run -p agent-k --bin test_case -- 0 --model kimi

use std::io::{self, BufRead, IsTerminal, Write};

use agent_k::agents::get_coworker_agent;
use ailoy::{
    agent::Agent,
    lang_model::LangModelAPISchema,
    message::{Message, Part, Role},
};
use futures::StreamExt;
use url::Url;

#[path = "test_case/cases.rs"]
mod cases;
use cases::{Case, get_coworker_cases};

const COWORKER_AGENT_NAME: &str = "minerva";
const COWORKER_AGENT_OPENAI_MODEL: &str = "openai/gpt-5.5";
const COWORKER_AGENT_CLAUDE_MODEL: &str = "anthropic/claude-opus-4-7";
const COWORKER_AGENT_GEMINI_MODEL: &str = "gemini/gemini-3.5-flash";
const COWORKER_AGENT_KIMI_MODEL: &str = "moonshot/kimi-k2.6";
const ARTIFACT_DIR: &str = "./artifacts";

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
    let mut case_no_arg: Option<&str> = None;
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
                if case_no_arg.is_some() {
                    anyhow::bail!("unexpected argument '{}'", s);
                }
                case_no_arg = Some(s);
                i += 1;
            }
        }
    }

    let case_no: usize = match case_no_arg {
        Some(s) => s.parse().map_err(|_| {
            anyhow::anyhow!(
                "invalid case number '{}', expected a non-negative integer",
                s
            )
        })?,
        None => {
            eprintln!("usage: test_case <case_no> [--model openai|claude|gemini|kimi]");
            std::process::exit(2);
        }
    };

    let coworker_agent_model = match model_arg {
        None | Some("openai") => COWORKER_AGENT_OPENAI_MODEL,
        Some("claude") => COWORKER_AGENT_CLAUDE_MODEL,
        Some("gemini") => COWORKER_AGENT_GEMINI_MODEL,
        Some("kimi") => COWORKER_AGENT_KIMI_MODEL,
        Some(other) => anyhow::bail!(
            "invalid --model '{}', expected 'openai', 'claude', 'gemini', or 'kimi'",
            other
        ),
    };

    let mut cases = get_coworker_cases();
    if case_no >= cases.len() {
        anyhow::bail!(
            "case {} out of range (have {} case(s))",
            case_no,
            cases.len()
        );
    }
    let case = cases.swap_remove(case_no);

    clean_artifact_dir();
    write_case_files(&case)?;

    let mut agent =
        get_coworker_agent(COWORKER_AGENT_NAME, coworker_agent_model, ARTIFACT_DIR).await?;
    println!(
        "[coworker] starting as '{}' ({}) — case #{}",
        COWORKER_AGENT_NAME, coworker_agent_model, case_no
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

fn clean_artifact_dir() {
    let path = std::path::Path::new(ARTIFACT_DIR);
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
    let base = std::path::Path::new(ARTIFACT_DIR);
    for (bytes, rel) in &case.files {
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
