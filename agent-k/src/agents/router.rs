use ailoy::{
    agent::AgentBuilder,
    message::{Message, Part, Role},
};
use serde::Deserialize;
use tokio_stream::StreamExt;

const ROUTER_INSTRUCTION: &str = r#"You are a router. Read the user's request and choose exactly one agent to handle it.

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

## Response format
{"agent": "<agent name>", "reason": "<one short sentence>"}
Respond with EXACTLY one JSON object, and nothing else (no prose, no markdown, no code fence). The "agent" field must be exactly one of the available agents.
"#;

#[derive(Debug, Deserialize)]
pub struct RouterDecision {
    pub agent: String,

    #[serde(default)]
    pub reason: Option<String>,
}

const ROUTER_MAX_RETRIES: usize = 2;

pub async fn run_gpt_router_agent(user_input: impl Into<String>) -> anyhow::Result<RouterDecision> {
    run_router_agent("openai/gpt-4o-mini", user_input).await
}

pub async fn run_claude_router_agent(
    user_input: impl Into<String>,
) -> anyhow::Result<RouterDecision> {
    run_router_agent("anthropic/claude-haiku-4-5", user_input).await
}

async fn run_router_agent(
    model: &str,
    user_input: impl Into<String>,
) -> anyhow::Result<RouterDecision> {
    let mut agent = AgentBuilder::new(model)
        .instruction(ROUTER_INSTRUCTION)
        .build()?;

    let mut next_message = Some(Message::new(Role::User).with_contents([Part::text(user_input)]));
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

        let mut it = serde_json::Deserializer::from_str(&raw).into_iter::<RouterDecision>();
        last_err = match it.next() {
            Some(Ok(d)) => return Ok(d),
            Some(Err(e)) => {
                format!("JSON parse failed: {e}")
            }
            None => "response had no JSON object".to_string(),
        };

        next_message = Some(Message::new(Role::User).with_contents([Part::text(format!(
            "Your previous response is not valid JSON. Respond again.\n\n{last_err}."
        ))]));
    }
    Ok(RouterDecision {
        agent: "minerva".to_string(),
        reason: Some(format!("fallback: {last_err}")),
    })
}

