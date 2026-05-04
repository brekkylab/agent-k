use ailoy::message::{Message, Part, Role};
use anyhow::Result;

use super::helper::HelperAgent;

/// Pulls a title out of a YAML frontmatter `title:` field or the first H1
/// heading. Returns `None` when neither is present — caller falls back to
/// `TitleAgent`.
fn extract_title(content: &str) -> Option<String> {
    if content.starts_with("---") {
        if let Some(end) = content[3..].find("\n---") {
            let frontmatter = &content[3..end + 3];
            for line in frontmatter.lines() {
                if let Some(rest) = line.strip_prefix("title:") {
                    let raw = rest.trim();
                    let title = if let Some(inner) =
                        raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
                    {
                        inner.replace("''", "'")
                    } else if let Some(inner) =
                        raw.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                    {
                        inner.to_string()
                    } else {
                        raw.to_string()
                    };
                    if !title.is_empty() {
                        return Some(title);
                    }
                }
            }
        }
    }

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            let title = rest.trim().to_string();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }

    None
}

const TITLE_PREVIEW_CHARS: usize = 8192;

const TITLE_INSTRUCTION: &str = concat!(
    "You are generating a title for a document. ",
    "Given document content, return ONLY a JSON object: ",
    "{\"result\": \"<string>\"}.\n\n",
    "title rules:\n",
    "- Concise, under 10 words.\n",
    "- Plain text. No markdown, no surrounding quotes inside the value.",
);

/// Generates a document title via LLM. Reads from ailoy's process-global
/// default provider.
struct TitleAgent;

impl HelperAgent for TitleAgent {
    type Input<'a> = &'a str;
    type Output = String;
    const INSTRUCTION: &'static str = TITLE_INSTRUCTION;

    fn build_query(&content: &&str) -> Message {
        let snippet: String = content.chars().take(TITLE_PREVIEW_CHARS).collect();
        Message::new(Role::User).with_contents([Part::text(snippet)])
    }

    fn fallback(_: &&str) -> String {
        "Untitled".to_string()
    }
}

/// Generates a title from `content`. The frontmatter/H1 fast path runs first;
/// if neither is present, falls back to `TitleAgent` (which substitutes
/// `"Untitled"` for an empty/malformed LLM response).
pub(super) async fn get_title(content: &str) -> Result<String> {
    if let Some(t) = extract_title(content) {
        return Ok(t);
    }
    TitleAgent::generate(content).await
}

const PURPOSE_INSTRUCTION: &str = concat!(
    "You are generating search metadata for a document retrieval system. ",
    "Your output will be used as BM25 search terms — optimize for retrieval, NOT readability.\n\n",
    "Given a document content preview (first 3000 characters), return ONLY a JSON object: ",
    "{\"result\": \"<string>\"}.\n\n",
    "purpose rules:\n",
    "- ONE sentence, 80–150 characters\n",
    "- MUST include: entity name(s), year/date, document type, 3–5 key topic terms\n",
    "- Think: \"what search queries should find this document?\"\n",
    "- Do NOT describe what the document says. Write what it IS and what it is FOR.\n\n",
    "GOOD: \"3M Company FY2018 10-K Annual Report — revenue $32.8B, safety industrial, healthcare, EPS growth\"\n",
    "BAD:  \"This document discusses the company's financial results\"",
);

const PURPOSE_PREVIEW_CHARS: usize = 3000;

/// Generates BM25-friendly search metadata for a document via LLM. Reads from
/// ailoy's process-global default provider.
struct PurposeAgent;

impl HelperAgent for PurposeAgent {
    type Input<'a> = &'a str;
    type Output = String;
    const INSTRUCTION: &'static str = PURPOSE_INSTRUCTION;

    fn build_query(&content: &&str) -> Message {
        let snippet: String = content.chars().take(PURPOSE_PREVIEW_CHARS).collect();
        Message::new(Role::User).with_contents([Part::text(snippet)])
    }

    /// Empty string is the documented fallback — `Store::ingest` indexes
    /// the document without purpose metadata in that case.
    fn fallback(_: &&str) -> String {
        String::new()
    }
}

/// Runs `PurposeAgent` over `content`. An empty/malformed LLM response is
/// substituted with `""` (which `Store::ingest` indexes as missing purpose
/// metadata).
pub(super) async fn get_purpose(content: &str) -> Result<String> {
    PurposeAgent::generate(content).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frontmatter_with_title(line: &str) -> String {
        format!("---\n{line}\n---\n\nbody\n")
    }

    #[test]
    fn extract_title_single_quoted_plain() {
        let doc = frontmatter_with_title("title: 'Hello world'");
        assert_eq!(extract_title(&doc).as_deref(), Some("Hello world"));
    }

    #[test]
    fn extract_title_single_quoted_with_apostrophe() {
        // `'` is escaped as `''` in YAML single-quoted style.
        let doc = frontmatter_with_title("title: 'Don''t stop'");
        assert_eq!(extract_title(&doc).as_deref(), Some("Don't stop"));
    }

    #[test]
    fn extract_title_single_quoted_with_double_quote_and_backslash() {
        // `"` and `\` pass through literally — no escaping in this style.
        let doc = frontmatter_with_title(r#"title: 'That "Smart" Move with C:\path'"#);
        assert_eq!(
            extract_title(&doc).as_deref(),
            Some(r#"That "Smart" Move with C:\path"#),
        );
    }

    #[test]
    fn extract_title_single_quoted_with_all_special_chars() {
        let doc = frontmatter_with_title(r#"title: 'Mix ''a'' "b" c\d'"#);
        assert_eq!(
            extract_title(&doc).as_deref(),
            Some(r#"Mix 'a' "b" c\d"#),
        );
    }

    #[test]
    fn extract_title_unquoted_returned_literally() {
        let doc = frontmatter_with_title("title: Plain Title");
        assert_eq!(extract_title(&doc).as_deref(), Some("Plain Title"));
    }

    #[test]
    fn extract_title_double_quoted_strips_outer_only() {
        // YAML-style outer `"` is treated as syntax (one stripped from each
        // side); backslash escapes inside are left literal.
        let doc = frontmatter_with_title(r#"title: "Hello \"World\"""#);
        assert_eq!(
            extract_title(&doc).as_deref(),
            Some(r#"Hello \"World\""#),
        );
    }

    #[test]
    fn extract_title_falls_back_to_h1_when_frontmatter_missing() {
        let doc = "# Heading Title\n\nbody\n";
        assert_eq!(extract_title(doc).as_deref(), Some("Heading Title"));
    }
}
