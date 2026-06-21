use ailoy::agent::{Agent, AgentBuilder, AgentProvider, default_provider};

use crate::agents::ExtraTools;

/// Buddy is the lightweight conversational surface and the project's
/// **app-control specialist**: brainstorming, casual chat, quick explanations,
/// translation — plus, when the host injects them, read-only tools to inspect
/// the app (automations, sessions, projects, members). Unlike
/// coworker/deep-research it runs without a sandbox — it owns no files and runs
/// no code — so it is cheap to spin up and answers directly (with web search
/// when a fact is needed).
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

/// Appended to the instruction when app-control tools are injected, so Buddy
/// knows it can act on the app itself.
pub const BUDDY_APP_TOOLS_NOTE: &str = r#"

## App control
You have tools to operate this app on the user's behalf:
- Automations: list/read them, read their runs and events, create and update
  them, add cron schedules, trigger a manual run, and delete them.
- Sessions: list and read chat sessions (read-only).
- Projects & members: list projects/members, add or remove members.
- Users: identify the current user and look people up.

These tools act with the permissions of the user who owns this session — you can
only do what that user could do. Never claim access you do not have; if a tool
reports a permission error, relay it plainly. Before any destructive or
hard-to-undo action (deleting an automation, removing a member), state exactly
what you will do and get the user's explicit confirmation first. After a
write, briefly confirm what changed.

Tool results include UUIDs for follow-up calls — use them to chain tool calls,
but don't list raw IDs in your reply unless the user explicitly asks. Refer to
things by name or title instead."#;

/// Build the Buddy agent. `name` is the agent's display identity; `model` is the
/// resolved `provider/model-id`. No sandbox, no filesystem — just chat plus an
/// optional web search. When `extra_tools` is supplied, its tool functions are
/// registered onto a clone of the default provider (so built-ins still resolve)
/// and its descriptions are advertised to the model.
pub fn get_buddy_agent(
    name: impl AsRef<str>,
    model: impl AsRef<str>,
    extra_tools: Option<ExtraTools>,
) -> anyhow::Result<Agent> {
    let mut inst = BUDDY_INSTRUCTION.replace("{{NAME}}", name.as_ref());
    let mut builder = AgentBuilder::new(model.as_ref());

    match extra_tools {
        Some(extra) if !extra.is_empty() => {
            inst.push_str(BUDDY_APP_TOOLS_NOTE);
            // Start from the default provider so built-ins (web search) keep
            // resolving, then let the host install its tool functions.
            let mut tools = default_provider().tools.clone();
            (extra.register)(&mut tools);
            let provider = AgentProvider {
                models: default_provider().models.clone(),
                tools,
            };
            builder = builder.provider(provider).tools(extra.descs);
        }
        _ => {}
    }

    builder
        .instruction(inst)
        .web_search_tool(vec![])
        .build()
}
