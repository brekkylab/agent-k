use ailoy::{
    agent::{Agent, AgentProvider, AgentSpec},
    message::{Message, Part, Role},
};
use anyhow::{Context as _, Result};
use futures::StreamExt as _;

fn parse_title(content: &str) -> Option<String> {
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

/// Wraps an Ailoy agent used to generate document titles via LLM.
pub struct TitleAgent {
    spec: AgentSpec,
    provider: Option<AgentProvider>,
}

impl TitleAgent {
    pub fn new(provider: Option<AgentProvider>) -> Self {
        Self {
            spec: AgentSpec::new("openai/gpt-5.4-mini").instruction(concat!(
                "You are a title generator. ",
                "Given document content, reply with only a concise title under 10 words.",
            )),
            provider,
        }
    }

    pub async fn generate(&self, content: &str) -> Result<String> {
        let snippet: String = content.chars().take(8192).collect();
        let query = Message::new(Role::User).with_contents([Part::text(snippet)]);

        let mut agent = match &self.provider {
            Some(provider) => Agent::try_with_provider(self.spec.clone(), provider).await?,
            None => Agent::try_new(self.spec.clone()).await?,
        };

        let mut text_parts: Vec<String> = Vec::new();
        {
            let mut stream = agent.run(query);
            while let Some(result) = stream.next().await {
                let output = result?;
                for part in &output.message.contents {
                    if let Some(text) = part.as_text() {
                        text_parts.push(text.to_string());
                    }
                }
            }
        }

        let title = text_parts.join("").trim().to_string();
        Ok(if title.is_empty() {
            "Untitled".to_string()
        } else {
            title
        })
    }
}

pub async fn get_title(content: &str) -> Result<String> {
    match parse_title(content) {
        Some(t) => Ok(t),
        None => {
            dotenvy::dotenv().ok();

            let mut provier = AgentProvider::new();
            provier.model_openai(
                std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY not set in environment")?,
            );
            TitleAgent::new(Some(provier)).generate(content).await
        }
    }
}

const PURPOSE_INSTRUCTION: &str = concat!(
    "You are generating search metadata for a document retrieval system. ",
    "Your output will be used as BM25 search terms — optimize for retrieval, NOT readability.\n\n",
    "Given a document content preview (first 3000 characters), return ONLY a JSON object: ",
    "{\"purpose\": \"<string>\"}.\n\n",
    "purpose rules:\n",
    "- ONE sentence, 80–150 characters\n",
    "- MUST include: entity name(s), year/date, document type, 3–5 key topic terms\n",
    "- Think: \"what search queries should find this document?\"\n",
    "- Do NOT describe what the document says. Write what it IS and what it is FOR.\n\n",
    "GOOD: \"3M Company FY2018 10-K Annual Report — revenue $32.8B, safety industrial, healthcare, EPS growth\"\n",
    "BAD:  \"This document discusses the company's financial results\"",
);

const PURPOSE_PREVIEW_CHARS: usize = 3000;

/// Wraps an Ailoy agent used to generate document purpose metadata via LLM.
pub struct PurposeAgent {
    spec: AgentSpec,
    provider: Option<AgentProvider>,
}

impl PurposeAgent {
    pub fn new(provider: Option<AgentProvider>) -> Self {
        Self {
            spec: AgentSpec::new("openai/gpt-5.4-mini").instruction(PURPOSE_INSTRUCTION),
            provider,
        }
    }

    pub async fn generate(&self, content: &str) -> Result<String> {
        let snippet: String = content.chars().take(PURPOSE_PREVIEW_CHARS).collect();
        let query = Message::new(Role::User).with_contents([Part::text(snippet)]);

        let mut agent = match &self.provider {
            Some(provider) => Agent::try_with_provider(self.spec.clone(), provider).await?,
            None => Agent::try_new(self.spec.clone()).await?,
        };

        let mut text_parts: Vec<String> = Vec::new();
        {
            let mut stream = agent.run(query);
            while let Some(result) = stream.next().await {
                let output = result?;
                for part in &output.message.contents {
                    if let Some(text) = part.as_text() {
                        text_parts.push(text.to_string());
                    }
                }
            }
        }

        let raw = text_parts.join("");
        Ok(parse_purpose_response(&raw))
    }
}

fn parse_purpose_response(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(p) = value.get("purpose").and_then(|v| v.as_str())
    {
        return p.trim().to_string();
    }

    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}'))
        && start < end
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end])
        && let Some(p) = value.get("purpose").and_then(|v| v.as_str())
    {
        return p.trim().to_string();
    }

    String::new()
}

pub async fn get_purpose(content: &str) -> Result<String> {
    dotenvy::dotenv().ok();

    let mut provider = AgentProvider::new();
    provider.model_openai(
        std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY not set in environment")?,
    );

    let purpose = PurposeAgent::new(Some(provider)).generate(content).await?;
    if purpose.is_empty() {
        log::warn!("purpose generation returned empty string; indexing without purpose metadata");
    }
    Ok(purpose)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_purpose_response_plain_json() {
        let raw = r#"{"purpose": "3M Company FY2018 10-K Annual Report"}"#;
        assert_eq!(
            parse_purpose_response(raw),
            "3M Company FY2018 10-K Annual Report"
        );
    }

    #[test]
    fn parse_purpose_response_with_whitespace() {
        let raw = "\n  {\"purpose\": \"hello\"}  \n";
        assert_eq!(parse_purpose_response(raw), "hello");
    }

    #[test]
    fn parse_purpose_response_with_surrounding_text() {
        let raw = "Sure, here you go: {\"purpose\": \"Costco 2023 Q1 earnings\"} — done.";
        assert_eq!(parse_purpose_response(raw), "Costco 2023 Q1 earnings");
    }

    #[test]
    fn parse_purpose_response_empty() {
        assert_eq!(parse_purpose_response(""), "");
        assert_eq!(parse_purpose_response("   "), "");
    }

    #[test]
    fn parse_purpose_response_invalid_json() {
        assert_eq!(parse_purpose_response("not json"), "");
        assert_eq!(parse_purpose_response("{not: json}"), "");
    }

    #[test]
    fn parse_purpose_response_missing_field() {
        let raw = r#"{"other": "value"}"#;
        assert_eq!(parse_purpose_response(raw), "");
    }

    fn frontmatter_with_title(line: &str) -> String {
        format!("---\n{line}\n---\n\nbody\n")
    }

    #[test]
    fn parse_title_single_quoted_plain() {
        let doc = frontmatter_with_title("title: 'Hello world'");
        assert_eq!(parse_title(&doc).as_deref(), Some("Hello world"));
    }

    #[test]
    fn parse_title_single_quoted_with_apostrophe() {
        // `'` is escaped as `''` in YAML single-quoted style.
        let doc = frontmatter_with_title("title: 'Don''t stop'");
        assert_eq!(parse_title(&doc).as_deref(), Some("Don't stop"));
    }

    #[test]
    fn parse_title_single_quoted_with_double_quote_and_backslash() {
        // `"` and `\` pass through literally — no escaping in this style.
        let doc = frontmatter_with_title(r#"title: 'That "Smart" Move with C:\path'"#);
        assert_eq!(
            parse_title(&doc).as_deref(),
            Some(r#"That "Smart" Move with C:\path"#),
        );
    }

    #[test]
    fn parse_title_single_quoted_with_all_special_chars() {
        let doc = frontmatter_with_title(r#"title: 'Mix ''a'' "b" c\d'"#);
        assert_eq!(
            parse_title(&doc).as_deref(),
            Some(r#"Mix 'a' "b" c\d"#),
        );
    }

    #[test]
    fn parse_title_unquoted_returned_literally() {
        let doc = frontmatter_with_title("title: Plain Title");
        assert_eq!(parse_title(&doc).as_deref(), Some("Plain Title"));
    }

    #[test]
    fn parse_title_double_quoted_strips_outer_only() {
        // YAML-style outer `"` is treated as syntax (one stripped from each
        // side); backslash escapes inside are left literal.
        let doc = frontmatter_with_title(r#"title: "Hello \"World\"""#);
        assert_eq!(
            parse_title(&doc).as_deref(),
            Some(r#"Hello \"World\""#),
        );
    }

    #[test]
    fn parse_title_falls_back_to_h1_when_frontmatter_missing() {
        let doc = "# Heading Title\n\nbody\n";
        assert_eq!(parse_title(doc).as_deref(), Some("Heading Title"));
    }
}
