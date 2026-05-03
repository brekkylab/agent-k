//! Generic LLM helper trait. Implement `HelperAgent` once per helper — the
//! trait supplies the dispatch boilerplate (agent construction, streaming,
//! text concatenation) via a default `generate` body. All helpers read from
//! ailoy's process-global default provider; populate it once at app boot via
//! `ailoy::agent::default_provider_mut`.

use ailoy::{
    agent::{Agent, AgentSpec},
    message::Message,
};
use anyhow::Result;
use futures::StreamExt as _;

/// Per-helper definition. Implement this on a unit struct, then call
/// `MyAgent::generate(input).await` directly.
pub(super) trait HelperAgent {
    /// Caller-supplied input. May borrow from the caller's stack.
    type Input<'a>;
    /// Parsed response.
    type Output;
    /// Model id, e.g. `"openai/gpt-5.4-mini"`.
    const MODEL: &'static str;
    /// System instruction for the agent.
    const INSTRUCTION: &'static str;

    /// Render `input` to a chat `Message` to send to the LLM.
    fn build_query(input: Self::Input<'_>) -> Message;
    /// Convert the joined raw text response into `Output`.
    fn parse(raw: &str) -> Self::Output;

    /// Dispatch a single LLM call against ailoy's process-global default
    /// provider, then run the response through `parse`.
    async fn generate(input: Self::Input<'_>) -> Result<Self::Output> {
        let spec = AgentSpec::new(Self::MODEL).instruction(Self::INSTRUCTION);
        let mut agent = Agent::try_new(spec).await?;
        let query = Self::build_query(input);

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
        Ok(Self::parse(&raw))
    }
}
