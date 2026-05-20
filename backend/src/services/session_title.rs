use std::time::Duration;

use ailoy::{
    agent::AgentBuilder,
    message::{Message, Part, Role},
};
use futures_util::StreamExt;

const TITLE_MODEL: &str = "anthropic/claude-haiku-4-5";
pub const TITLE_MAX_LEN: usize = 60;
const TITLE_TIMEOUT_SECS: u64 = 15;

/// Generate a one-sentence title for a session from the first user message.
/// Falls back to the first `TITLE_MAX_LEN` characters of the message on any error.
pub async fn generate_session_title(first_user_text: &str) -> String {
    let result: Result<Result<String, String>, _> = tokio::time::timeout(
        Duration::from_secs(TITLE_TIMEOUT_SECS),
        call_llm_for_session_title(first_user_text),
    )
    .await;

    match result {
        Ok(Ok(title)) if !title.trim().is_empty() => sanitize_session_title(&title),
        _ => sanitize_session_title(first_user_text),
    }
}

async fn call_llm_for_session_title(text: &str) -> Result<String, String> {
    let mut agent = AgentBuilder::new(TITLE_MODEL)
        .instruction(
            format!("You are a concise title generator. \
             Respond with a single short phrase (under {TITLE_MAX_LEN} characters) that captures the topic. \
             No quotes, no trailing punctuation."),
        )
        .build()
        .map_err(|e| e.to_string())?;

    let msg = Message::new(Role::User).with_contents([Part::text(text)]);
    let mut run = agent.run(msg);
    let mut parts: Vec<String> = Vec::new();

    while let Some(item) = run.next().await {
        let output = item.map_err(|e| e.to_string())?;
        for part in &output.message.contents {
            if let Some(t) = part.as_text() {
                parts.push(t.to_string());
            }
        }
    }

    Ok(parts.join(""))
}

fn sanitize_session_title(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(TITLE_MAX_LEN)
        .collect()
}
