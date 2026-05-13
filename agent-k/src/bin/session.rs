//! Session CLI.
//!
//! Reads a user request from argv (or stdin if none given), asks a small router
//! agent which sub-agent should handle it, then dispatches.
//!
//! echo "주간 보고 메일 초안 써줘" | cargo run -p agent-k --bin session
//! cargo run -p agent-k --bin session -- "세계 날씨를 확인할 수 있는 간단한 HTML 페이지 만들어주세요"

use std::io::{self, BufRead, IsTerminal, Read, Write};

use agent_k::agents::{get_gpt_minerva_agent, run_gpt_router_agent};
use ailoy::{
    agent::Agent,
    message::{Message, Part, Role},
};
use futures::StreamExt;

enum InputSource {
    Stdin,
    Tty(io::BufReader<std::fs::File>),
}

enum Session {
    Speedwagon,
    Vegapunk,
    Minerva(Agent),
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
            eprintln!("[info] empty input, nothing to do");
            None
        } else {
            Some(s)
        }
    } else {
        None
    };

    let mut session: Option<Session> = None;
    if let Some(input) = first_input {
        match route_and_run(&input).await {
            Ok(s) => session = Some(s),
            Err(e) => eprintln!("[error] {e}"),
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
            eprintln!();
            return Ok(());
        }
        let user_input = buf.trim().to_string();
        if user_input.is_empty() {
            continue;
        }
        if let Some(cmd) = user_input.strip_prefix('/') {
            match handle_slash(cmd) {
                SlashAction::Unroute => {
                    session = None;
                    eprintln!("[session] cleared. next message will be re-routed.");
                }
                SlashAction::Force(new_session) => {
                    session = Some(new_session);
                    eprintln!("[session] forced.");
                }
                SlashAction::Help => {
                    eprintln!(
                        "[help] /route — clear session, re-route on next message\n\
                         [help] /minerva | /speedwagon | /vegapunk — force a session\n\
                         [help] /help — this message"
                    );
                }
                SlashAction::Unknown(c) => {
                    eprintln!("[error] unknown slash command: /{c}");
                }
                SlashAction::Error(msg) => {
                    eprintln!("[error] {msg}");
                }
            }
            continue;
        }
        let result = match session.as_mut() {
            None => match route_and_run(&user_input).await {
                Ok(s) => {
                    session = Some(s);
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Some(s) => run_in_session(s, &user_input).await,
        };
        if let Err(e) = result {
            eprintln!("[error] {e}");
        }
    }
}

enum SlashAction {
    Unroute,
    Force(Session),
    Help,
    Unknown(String),
    Error(String),
}

fn handle_slash(cmd: &str) -> SlashAction {
    let name = cmd.trim();
    match name {
        "route" => SlashAction::Unroute,
        "speedwagon" => SlashAction::Force(Session::Speedwagon),
        "vegapunk" => SlashAction::Force(Session::Vegapunk),
        "minerva" => match build_minerva() {
            Ok(a) => SlashAction::Force(Session::Minerva(a)),
            Err(e) => SlashAction::Error(format!("/minerva: build failed: {e}")),
        },
        "help" | "?" => SlashAction::Help,
        other => SlashAction::Unknown(other.to_string()),
    }
}

async fn route_and_run(user_input: &str) -> anyhow::Result<Session> {
    let decision = run_gpt_router_agent(user_input).await?;

    println!(
        "[router] {}",
        serde_json::to_string(&serde_json::json!({
            "agent": decision.agent,
            "reason": decision.reason,
        }))?
    );

    let mut session = match decision.agent.as_str() {
        "speedwagon" => Session::Speedwagon,
        "vegapunk" => Session::Vegapunk,
        "minerva" => Session::Minerva(build_minerva()?),
        other => anyhow::bail!("router returned unknown agent '{other}'"),
    };
    run_in_session(&mut session, user_input).await?;
    Ok(session)
}

async fn run_in_session(session: &mut Session, user_input: &str) -> anyhow::Result<()> {
    match session {
        Session::Speedwagon => {
            eprintln!("[dispatch] TODO: speedwagon (RAG Q&A) is not implemented yet");
            eprintln!("[dispatch] forwarding query: {}", cap_for_echo(user_input));
            Ok(())
        }
        Session::Vegapunk => {
            eprintln!("[dispatch] TODO: vegapunk (deep research) is not implemented yet");
            eprintln!("[dispatch] forwarding query: {}", cap_for_echo(user_input));
            Ok(())
        }
        Session::Minerva(agent) => stream_minerva_turn(agent, user_input).await,
    }
}

const DISPATCH_ECHO_MAX: usize = 200;

fn cap_for_echo(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= DISPATCH_ECHO_MAX {
        return s.to_string();
    }
    let head: String = chars[..DISPATCH_ECHO_MAX].iter().collect();
    format!("{head}… (+{} chars)", chars.len() - DISPATCH_ECHO_MAX)
}

fn clean_artifact_dir() {
    let path = std::path::Path::new(".artifact");
    if !path.exists() {
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(path) {
        eprintln!("[warn] failed to clean {}: {e}", path.display());
    }
}

fn build_minerva() -> anyhow::Result<Agent> {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".into());
    let agent = get_gpt_minerva_agent(std::env::consts::OS, &cwd)?;
    println!("[minerva] starting");
    Ok(agent)
}

async fn stream_minerva_turn(agent: &mut Agent, user_input: &str) -> anyhow::Result<()> {
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
                            print!("{t}");
                            io::stdout().flush().ok();
                        }
                    }
                }
                if let Some(tcs) = &msg.tool_calls {
                    for tc in tcs {
                        if let Some((_id, name, args)) = tc.as_function() {
                            let args_json = serde_json::to_string(args)
                                .unwrap_or_else(|_| "<unprintable>".into());
                            println!("[minerva] tool: {name} {args_json}");
                        }
                    }
                }
            }
            Role::Tool => {
                println!("[minerva] tool result");
            }
            _ => {}
        }
    }
    println!();
    Ok(())
}
