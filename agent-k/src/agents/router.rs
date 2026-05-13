use ailoy::{
    agent::AgentBuilder,
    message::{Message, Part, Role},
};
use serde::Deserialize;
use tokio_stream::StreamExt;

const ROUTER_INSTRUCTION: &str = r#"You are a router. Read the user's request, split it into one or more sub-requests, and choose which agent should handle each.

## Agents
- "speedwagon": RAG Q&A. Factual or knowledge questions answerable from a static document corpus.
- "vegapunk": Deep research. Multi-source investigation: literature review, topic survey, option comparison, or a long-form research report.
- "minerva": General-purpose execution. Running commands, exploring or editing files/code, orchestrating multi-step work, fetching live information, producing code or runnable artifacts.

## Rules
- Live information (today's weather, current stock price, today's news, anything that needs to be fetched right now) must route to minerva. speedwagon only covers static corpus knowledge. "As of <past date>" is a static fact, not live — route those to speedwagon.
- If the request asks for both analysis/comparison and a concrete artifact (code, config, script, runnable example), the artifact intent wins — route to minerva.
- If the request is primarily a question but also asks for an example, snippet, or code, treat the artifact intent as decisive and route to minerva.
- Requests to translate text from one specific language to another (e.g. "translate this Korean to English") are execution — route to minerva. Just writing in a non-English language is NOT a translation request.
- If the request does not fit any agent well (greetings, identity questions about yourself, pure noise, ambiguous fragments, refusals to act), still pick the closest agent but prefix the "reason" field with "fallback: ".
- Write "reason" in the dominant language of the user's request — the language carrying the semantic content, not short carrier phrases like "please" or "tell me".

## Splitting
- If the request contains MULTIPLE distinct intents (Q&A + execution, research + code, two unrelated questions, etc.), produce ONE step per intent, in the order the user wrote them.
- If the request is a SINGLE intent (even if listy or long), produce ONE step.
- Each step.input must be a SELF-CONTAINED rewriting of that slice of the user's request — the dispatcher will pass step.input to the chosen agent. If a later step depends on the previous step's result, write step.input so that it can be answered given "the previous result" (which the dispatcher will provide as context).
- Do NOT split a single research/survey/comparison task into per-item steps just because it lists multiple items.
- A request that asks for an explanation WITH a code example is ONE minerva step (artifact wins).
- Negations or self-corrections must be honored: only emit steps for what the user actually wants done.

## Response format
{"steps": [{"agent": "<agent name>", "input": "<sub-request>", "reason": "<one short sentence>"}]}
Respond with EXACTLY one JSON object, and nothing else (no prose, no markdown, no code fence). The "agent" field must be exactly one of the available agents.
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
    run_router_agent("openai/gpt-4o-mini", user_input).await
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
