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
             Summarize the user's request as a single short title phrase (under {TITLE_MAX_LEN} characters). \
             Output ONLY the title, on ONE line, as plain text: \
             no markdown, no formatting, no headings, no quotes, no trailing punctuation. \
             Do NOT answer the question or add any extra lines."
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

/// Reduce a raw model (or fallback) string to a clean one-line title.
///
/// Titles are a single line, so only the first non-empty line is kept — models
/// occasionally tack on a second line (a section header, or the answer itself).
/// Markdown emphasis/heading markers are turned into spaces (so removing `**`
/// doesn't glue two words together), non-whitespace control chars are dropped,
/// internal whitespace is collapsed, and the result is clamped to
/// [`TITLE_MAX_LEN`] characters.
fn sanitize_session_title(s: &str) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let cleaned: String = line
        .chars()
        .map(|c| if matches!(c, '*' | '_' | '`' | '#') { ' ' } else { c })
        .filter(|c| !(c.is_control() && !c.is_whitespace()))
        .collect();
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
        // Bell is dropped; the tab collapses to a single space.
        assert_eq!(sanitize_session_title("a\tb\u{7}c"), "a bc");
    }

    #[test]
    fn sanitize_keeps_only_first_line() {
        assert_eq!(
            sanitize_session_title("Walking Benefits Research Report\n**Physical Health Benefits**"),
            "Walking Benefits Research Report"
        );
    }

    #[test]
    fn sanitize_strips_markdown_without_gluing_words() {
        assert_eq!(sanitize_session_title("**Bold Title**"), "Bold Title");
        assert_eq!(sanitize_session_title("Report**Physical"), "Report Physical");
        assert_eq!(sanitize_session_title("# Heading"), "Heading");
    }
}
