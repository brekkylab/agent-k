use std::time::Duration;

use ailoy::{
    agent::AgentBuilder,
    message::{Message, Part, Role},
};
use futures_util::StreamExt;

const TITLE_MODEL: &str = "openai/gpt-5-nano";
const TITLE_FALLBACK_LEN: usize = 50;
const TITLE_TIMEOUT_SECS: u64 = 15;

/// Generate a one-sentence title for a session from the first user message.
/// Falls back to the first `TITLE_FALLBACK_LEN` characters on any error.
pub async fn generate_title(first_user_text: &str) -> String {
    let fallback = || {
        first_user_text
            .chars()
            .take(TITLE_FALLBACK_LEN)
            .collect::<String>()
    };

    let result: Result<Result<String, String>, _> = tokio::time::timeout(
        Duration::from_secs(TITLE_TIMEOUT_SECS),
        call_llm_for_title(first_user_text),
    )
    .await;

    match result {
        Ok(Ok(title)) if !title.trim().is_empty() => sanitize_title(&title),
        _ => fallback(),
    }
}

async fn call_llm_for_title(text: &str) -> Result<String, String> {
    let mut agent = AgentBuilder::new(TITLE_MODEL)
        .instruction(
            "You are a concise title generator. \
             Respond with a single short phrase (under 60 characters) that captures the topic. \
             No quotes, no trailing punctuation.",
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

fn sanitize_title(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(80)
        .collect()
}
