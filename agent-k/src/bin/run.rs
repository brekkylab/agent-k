//! Run CLI.
//!
//! Interactive session that talks to a single `coworker` agent. Unlike
//! `session`, there is no router — every turn is dispatched to the same
//! coworker agent running against a local `RunEnv`.
//!
//! echo "안녕" | cargo run -p agent-k --bin run
//! cargo run -p agent-k --bin run -- "현재 디렉토리를 정리해줘"

use std::io::{self, BufRead, IsTerminal, Read, Write};

use agent_k::agents::get_coworker_agent;
use ailoy::{
    agent::Agent,
    message::{Message, Part, Role},
};
use futures::StreamExt;

const COWORKER_AGENT_NAME: &str = "minerva";
const COWORKER_AGENT_MODEL: &str = "openai/gpt-5.4";
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
    let stdin_is_tty = io::stdin().is_terminal();

    let first_input = if !argv.is_empty() {
        let s = argv.join(" ").trim().to_string();
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
        get_coworker_agent(COWORKER_AGENT_NAME, COWORKER_AGENT_MODEL, ARTIFACT_DIR).await?;
    println!(
        "[coworker] starting as '{}' ({})",
        COWORKER_AGENT_NAME, COWORKER_AGENT_MODEL
    );

    if let Some(input) = first_input {
        if let Err(e) = stream_turn(&mut agent, &input).await {
            println!("[error] {e}");
        }
    }

    let mut source = if stdin_is_tty {
        InputSource::Stdin
    } else {
        match std::fs::File::open("/dev/tty") {
            Ok(f) => InputSource::Tty(io::BufReader::new(f)),
            Err(_) => return Ok(()),
        }
    };

    loop {
        eprint!("> ");
        io::stderr().flush().ok();
        let mut buf = String::new();
        let n = match &mut source {
            InputSource::Stdin => io::stdin().read_line(&mut buf)?,
            InputSource::Tty(r) => r.read_line(&mut buf)?,
        };
        if n == 0 {
            println!();
            return Ok(());
        }
        let user_input = buf.trim().to_string();
        if user_input.is_empty() {
            continue;
        }
        if let Err(e) = stream_turn(&mut agent, &user_input).await {
            println!("[error] {e}");
        }
    }
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
