//! Run CLI.
//!
//! Interactive session that talks to a single `coworker` agent. Unlike
//! `session`, there is no router — every turn is dispatched to the same
//! coworker agent running against a local `RunEnv`.
//!
//! cargo run -p agent-k --bin run -- "Hello"

use std::io::{self, BufRead, IsTerminal, Read, Write};

use agent_k::agents::get_coworker_agent;
use ailoy::{
    agent::Agent,
    message::{Message, Part, Role},
};
use futures::StreamExt;

const COWORKER_AGENT_NAME: &str = "minerva";
const COWORKER_AGENT_OPENAI_MODEL: &str = "openai/gpt-5.4";
const COWORKER_AGENT_CLAUDE_MODEL: &str = "anthropic/claude-opus-4-7";
const COWORKER_AGENT_GEMINI_MODEL: &str = "gemini/gemini-3.5-flash";
const ARTIFACT_DIR: &str = "./artifacts";

enum InputSource {
    Stdin,
    Tty(io::BufReader<std::fs::File>),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    clean_artifact_dir();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut model_arg: Option<String> = None;
    let mut prompt_args: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "--model" | "-m" => {
                let v = argv.get(i + 1).ok_or_else(|| {
                    anyhow::anyhow!("--model requires a value (openai|claude|gemini)")
                })?;
                model_arg = Some(v.clone());
                i += 2;
            }
            s if s.starts_with("--model=") => {
                model_arg = Some(s["--model=".len()..].to_string());
                i += 1;
            }
            _ => {
                prompt_args.push(argv[i].clone());
                i += 1;
            }
        }
    }

    let coworker_agent_model = match model_arg.as_deref() {
        None | Some("openai") => COWORKER_AGENT_OPENAI_MODEL,
        Some("claude") | Some("anthropic") => COWORKER_AGENT_CLAUDE_MODEL,
        Some("gemini") | Some("google") => COWORKER_AGENT_GEMINI_MODEL,
        Some(other) => anyhow::bail!(
            "invalid --model '{}', expected 'openai', 'claude', or 'gemini'",
            other
        ),
    };

    let stdin_is_tty = io::stdin().is_terminal();

    let first_input = if !prompt_args.is_empty() {
        let s = prompt_args.join(" ").trim().to_string();
        (!s.is_empty()).then_some(s)
    } else if !stdin_is_tty {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        let s = buf.trim().to_string();
        if s.is_empty() {
            println!("[info] empty input, nothing to do");
            None
        } else {
            Some(s)
        }
    } else {
        None
    };

    let mut agent =
        get_coworker_agent(COWORKER_AGENT_NAME, coworker_agent_model, ARTIFACT_DIR).await?;
    println!(
        "[coworker] starting as '{}' ({})",
        COWORKER_AGENT_NAME, coworker_agent_model
    );

    if let Some(input) = first_input {
        if let Err(e) = stream_turn(&mut agent, &input).await {
            println!("[error] {e}");
        }
    }

    let source = if stdin_is_tty {
        InputSource::Stdin
    } else {
        match std::fs::File::open("/dev/tty") {
            Ok(f) => InputSource::Tty(io::BufReader::new(f)),
            Err(_) => return Ok(()),
        }
    };

    // Spawn a dedicated OS thread for blocking line reads so the tokio runtime
    // stays free to handle ctrl_c while waiting at the prompt.
    // req_tx/req_rx: main signals the thread to print the prompt and read a line.
    // line_tx/line_rx: thread sends the line (or None on EOF/error) back to main.
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
                            if let Err(e) = stream_turn(&mut agent, &input).await {
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

async fn stream_turn(agent: &mut Agent, user_input: &str) -> anyhow::Result<()> {
    let query = Message::new(Role::User).with_contents([Part::text(user_input)]);
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
