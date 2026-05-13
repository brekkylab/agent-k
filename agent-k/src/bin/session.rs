//! Session CLI.
//!
//! Reads a user request from argv (or stdin if none given), asks a small router
//! agent for a Plan (a sequence of (agent, input) steps), then dispatches each
//! step in order. Step outputs are appended verbatim to stdout (streaming for
//! minerva). Dependent steps see prior step outputs in their prompt.
//!
//! echo "주간 보고 메일 초안 써줘" | cargo run -p agent-k --bin session
//! cargo run -p agent-k --bin session -- "세계 날씨를 확인할 수 있는 간단한 HTML 페이지 만들어주세요"

use std::io::{self, BufRead, IsTerminal, Read, Write};

use agent_k::agents::{get_gpt_minerva_agent, run_gpt_router_agent, Plan};
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
            Some(s) => run_in_session(s, &user_input).await.map(|_| ()),
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
    let plan = run_gpt_router_agent(user_input).await?;

    println!(
        "[router] {}",
        serde_json::to_string(&plan_log(&plan))?
    );

    // Surface "current session" hint is the *first* step's agent. This keeps
    // the existing /minerva, /speedwagon, /vegapunk slash UX meaningful: a
    // user can see / force the agent that handled the head of the plan.
    let first_agent = plan.steps[0].agent.clone();
    let mut session = match first_agent.as_str() {
        "speedwagon" => Session::Speedwagon,
        "vegapunk" => Session::Vegapunk,
        "minerva" => Session::Minerva(build_minerva()?),
        other => anyhow::bail!("router returned unknown agent '{other}'"),
    };

    // Run each step in order. Prior step outputs are prepended to subsequent
    // step inputs so dependent intents can reference them. For single-step
    // plans, forward the user's original input as-is to preserve their
    // phrasing instead of using the router's paraphrase.
    let single_step = plan.steps.len() == 1;
    let mut accumulated: Vec<String> = Vec::with_capacity(plan.steps.len());
    for (i, step) in plan.steps.iter().enumerate() {
        let step_input = if single_step { user_input } else { &step.input };
        let prompt = if accumulated.is_empty() {
            step_input.to_string()
        } else {
            let mut s = String::from("Previous step results (chronological):\n");
            for (j, prev) in accumulated.iter().enumerate() {
                s.push_str(&format!("[step {}] {}\n\n", j + 1, prev));
            }
            s.push_str("---\nCurrent step: ");
            s.push_str(step_input);
            s
        };

        eprintln!(
            "[step {} • {}] {}",
            i + 1,
            step.agent,
            cap_for_echo(step_input)
        );

        // Reuse the head session for matching steps; otherwise spin up a
        // temporary session for that step.
        let out = if step.agent == first_agent {
            run_in_session(&mut session, &prompt).await?
        } else {
            let mut tmp = match step.agent.as_str() {
                "speedwagon" => Session::Speedwagon,
                "vegapunk" => Session::Vegapunk,
                "minerva" => Session::Minerva(build_minerva()?),
                other => anyhow::bail!("step {} agent unknown: {other}", i + 1),
            };
            run_in_session(&mut tmp, &prompt).await?
        };

        accumulated.push(out);
    }

    Ok(session)
}

fn plan_log(plan: &Plan) -> serde_json::Value {
    serde_json::json!({
        "steps": plan
            .steps
            .iter()
            .map(|s| serde_json::json!({
                "agent": s.agent,
                "input": s.input,
                "reason": s.reason,
            }))
            .collect::<Vec<_>>(),
    })
}

async fn run_in_session(session: &mut Session, user_input: &str) -> anyhow::Result<String> {
    match session {
        Session::Speedwagon => {
            eprintln!("[dispatch] TODO: speedwagon (RAG Q&A) is not implemented yet");
            eprintln!("[dispatch] forwarding query: {}", cap_for_echo(user_input));
            Ok("[speedwagon stub: not yet implemented]".to_string())
        }
        Session::Vegapunk => {
            eprintln!("[dispatch] TODO: vegapunk (deep research) is not implemented yet");
            eprintln!("[dispatch] forwarding query: {}", cap_for_echo(user_input));
            Ok("[vegapunk stub: not yet implemented]".to_string())
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

async fn stream_minerva_turn(agent: &mut Agent, user_input: &str) -> anyhow::Result<String> {
    let query = Message::new(Role::User).with_contents([Part::text(user_input)]);
    let mut stream = agent.run(query);
    let mut captured = String::new();
    while let Some(event) = stream.next().await {
        let event = event?;
        let msg = &event.message;
        match msg.role {
            Role::Assistant => {
                for part in &msg.contents {
                    if let Some(t) = part.as_text() {
                        if !t.is_empty() {
                            print!("{t}");
                            captured.push_str(t);
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
    Ok(captured)
}
