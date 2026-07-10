use std::time::Duration;

use ailoy::{
    agent::AgentBuilder,
    message::{Message, Part, Role},
};
use futures_util::StreamExt as _;

pub const TITLE_MAX_LEN: usize = 60;
const TITLE_TIMEOUT_SECS: u64 = 15;

/// Model used to summarise the first user turn into a session title. A cheap,
/// fast model is enough — the output is a short phrase, not an answer.
///
/// v2 has no runtime model-chain resolution (see `backend`'s
/// `resolve_title_model`), so the model is pinned here.
const TITLE_MODEL: &str = "anthropic/claude-haiku-4-5";

/// Generate a one-line title for a session from its first user message.
///
/// Falls back to the first [`TITLE_MAX_LEN`] characters of the message on any
/// error or timeout, so a title is always produced.
pub async fn generate_session_title(first_user_text: &str) -> String {
    let result = tokio::time::timeout(
        Duration::from_secs(TITLE_TIMEOUT_SECS),
        call_llm_for_session_title(first_user_text),
    )
    .await;

    match result {
        Ok(Ok(title)) if !title.trim().is_empty() => sanitize_session_title(&title),
        Ok(Ok(_)) => sanitize_session_title(first_user_text),
        Ok(Err(e)) => {
            tracing::warn!("session title generation failed ({e}); using first message");
            sanitize_session_title(first_user_text)
        }
        Err(_) => {
            tracing::warn!(
                "session title generation timed out after {TITLE_TIMEOUT_SECS}s; using first message"
            );
            sanitize_session_title(first_user_text)
        }
    }
}

async fn call_llm_for_session_title(text: &str) -> Result<String, String> {
    let mut agent = AgentBuilder::new(TITLE_MODEL)
        .instruction(format!(
            "You are a concise title generator. \
             Try to summarize user's question in a single short phrase (under {TITLE_MAX_LEN} characters). \
             Just summarize it, DO NOT try to generate the answer of the question. \
             No quotes, no trailing punctuation."
        ))
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

/// Strip control characters and clamp to [`TITLE_MAX_LEN`] characters.
fn sanitize_session_title(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(TITLE_MAX_LEN)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_trims_and_clamps() {
        let long = "a".repeat(100);
        assert_eq!(sanitize_session_title(&long).chars().count(), TITLE_MAX_LEN);
        assert_eq!(sanitize_session_title("  hello\n"), "hello");
    }

    #[test]
    fn sanitize_drops_control_chars() {
        assert_eq!(sanitize_session_title("a\tb\u{7}c"), "abc");
    }
}
