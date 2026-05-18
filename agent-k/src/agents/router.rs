use ailoy::{
    agent::AgentBuilder,
    message::{Message, Part, Role},
};
use serde::Deserialize;
use tokio_stream::StreamExt;

const ROUTER_INSTRUCTION: &str = r#"You are a router. Read the user's request and produce a plan of one or more steps, each routed to a single agent.

## Agents
- "speedwagon": Q&A. Factual or knowledge questions, regardless of how the answer is sourced.
- "vegapunk": Deep research. Multi-source investigation: literature review, topic survey, option comparison, or a long-form research report.
- "minerva": General-purpose execution. Running commands, exploring or editing files/code, orchestrating multi-step work, producing code or runnable artifacts.

## Rules
- A request that mixes Q&A/research with execution should split — speedwagon (or vegapunk) for the Q&A or research part, minerva for the execution part.
- Requests to translate text from one specific language to another (e.g. "translate this Korean to English") are execution — route to minerva. Just writing in a non-English language is not a translation request.
- If the request does not fit any agent well (greetings, identity questions about yourself, pure noise, ambiguous fragments, refusals to act), still pick the closest agent but prefix the "reason" field with "fallback: ".
- Write "reason" in the dominant language of the user's request — the language carrying the semantic content, not short carrier phrases like "please" or "tell me".

## Splitting
- One step per distinct intent, in the order the user wrote them. A single intent — even if listy, long, or about multiple items — stays one step.
- Each step.input is the corresponding part of the user's request, kept close to the original wording. The dispatcher passes step.input to the agent, with prior step outputs prepended as context, so phrase step.input as if those prior outputs are already in view.
- An explanation paired with a tiny inline example is one minerva step (artifact wins) — the general split rule above does not apply when the example is inline.
- Honor negations and self-corrections: only emit steps for the user's latest stated intent.

## Response format
{"steps": [{"agent": "<agent name>", "input": "<sub-request>", "reason": "<one short sentence>"}]}
Respond with exactly one JSON object, and nothing else (no prose, no markdown, no code fence). The "agent" field must be exactly one of the available agents.
"#;

#[derive(Debug, Deserialize)]
pub struct Step {
    pub agent: String,
    pub input: String,

    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Plan {
    pub steps: Vec<Step>,
}

const ROUTER_MAX_RETRIES: usize = 2;

pub async fn run_gpt_router_agent(user_input: impl Into<String>) -> anyhow::Result<Plan> {
    run_router_agent("openai/gpt-5.4-nano", user_input).await
}

pub async fn run_claude_router_agent(user_input: impl Into<String>) -> anyhow::Result<Plan> {
    run_router_agent("anthropic/claude-haiku-4-5", user_input).await
}

async fn run_router_agent(
    model: &str,
    user_input: impl Into<String>,
) -> anyhow::Result<Plan> {
    let user_input: String = user_input.into();
    let mut agent = AgentBuilder::new(model)
        .instruction(ROUTER_INSTRUCTION)
        .build()?;

    let mut next_message =
        Some(Message::new(Role::User).with_contents([Part::text(user_input.clone())]));
    let mut last_err = String::from("no attempts made");

    for _ in 0..ROUTER_MAX_RETRIES {
        let msg = next_message
            .take()
            .expect("next_message set before each iteration");
        let mut stream = agent.run(msg);
        while let Some(event) = stream.next().await {
            let _ = event?;
        }
        drop(stream);

        let last = agent
            .get_history()
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .ok_or_else(|| anyhow::anyhow!("router produced no assistant message"))?;

        let raw = last
            .contents
            .iter()
            .filter_map(|p| p.as_text())
            .collect::<Vec<_>>()
            .join("");

        let mut it = serde_json::Deserializer::from_str(&raw).into_iter::<Plan>();
        last_err = match it.next() {
            Some(Ok(plan)) => {
                if plan.steps.is_empty() {
                    "empty steps array".to_string()
                } else {
                    let invalid = plan.steps.iter().enumerate().find_map(|(i, s)| {
                        if !matches!(s.agent.as_str(), "speedwagon" | "vegapunk" | "minerva") {
                            Some(format!(
                                "invalid agent '{}' at step {}; must be one of speedwagon/vegapunk/minerva",
                                s.agent, i
                            ))
                        } else {
                            None
                        }
                    });
                    if let Some(msg) = invalid {
                        msg
                    } else {
                        return Ok(plan);
                    }
                }
            }
            Some(Err(e)) => format!("JSON parse failed: {e}"),
            None => "response had no JSON object".to_string(),
        };

        next_message = Some(Message::new(Role::User).with_contents([Part::text(format!(
            "Your previous response is not valid JSON. Respond again.\n\n{last_err}."
        ))]));
    }
    Ok(Plan {
        steps: vec![Step {
            agent: "minerva".to_string(),
            input: user_input,
            reason: Some(format!("fallback: {last_err}")),
        }],
    })
}
