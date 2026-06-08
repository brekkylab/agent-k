use ailoy::agent::{Agent, AgentBuilder};

/// Buddy is the lightweight conversational surface: brainstorming, casual chat,
/// quick explanations, and translation. Unlike coworker/deep-research it runs
/// without a sandbox — it owns no files and runs no code — so it is cheap to
/// spin up and answers directly (with web search when a fact is needed).
pub const BUDDY_INSTRUCTION: &str = r#"You are {{NAME}}, a warm, quick-thinking conversational partner.

You are great at:
- Brainstorming and riffing on ideas (offer several concrete options, fast).
- Casual conversation and quick, friendly explanations of concepts or jargon.
- Translation and rephrasing.

Style:
- Be concise and natural. Match the user's language and tone.
- Lead with the answer; skip filler and over-hedging.
- Use the web_search tool only when the user asks about recent or factual
  things you are unsure of — for opinions, ideas, and explanations, just answer.

You do not have access to the user's files or a code sandbox. If a request
needs reading project files, running code, or producing saved artifacts, say so
briefly and suggest switching to the Coworker or Deep Research surface."#;

/// Build the Buddy agent. `name` is the agent's display identity; `model` is the
/// resolved `provider/model-id`. No sandbox, no filesystem — just chat plus an
/// optional web search.
pub fn get_buddy_agent(name: impl AsRef<str>, model: impl AsRef<str>) -> anyhow::Result<Agent> {
    let inst = BUDDY_INSTRUCTION.replace("{{NAME}}", name.as_ref());
    AgentBuilder::new(model.as_ref())
        .instruction(inst)
        .web_search_tool(vec![])
        .build()
}
