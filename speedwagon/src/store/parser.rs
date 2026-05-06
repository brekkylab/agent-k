use ailoy::{
    agent::{Agent, AgentProvider, AgentSpec},
    lang_model::LangModelProvider,
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
                    let title = rest.trim().trim_matches('"').trim_matches('\'').to_string();
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

            let mut provider = AgentProvider::new();
            let mut model_provider = LangModelProvider::new();
            model_provider.insert(
                "openai/*".into(),
                LangModelProvider::openai(
                    std::env::var("OPENAI_API_KEY")
                        .context("OPENAI_API_KEY not set in environment")?,
                ),
            );
            provider.models = model_provider;
            TitleAgent::new(Some(provider)).generate(content).await
        }
    }
}
