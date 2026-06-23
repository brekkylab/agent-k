//! Read-only session tools: list sessions and read their message history.
//! No tool sends messages, starts runs, or creates/deletes sessions — by
//! design (prevents agent-triggers-agent loops and self-session deadlock).
//!
//! Every result flags whether a session is the **current** one — the session
//! this agent is itself running in — so the agent doesn't confuse "the chat I'm
//! in right now" with the others it can see.

use std::sync::Arc;

use ailoy::{to_value, tool::ToolDescBuilder, tool_func};
use serde_json::json;

use crate::app_tools::context::{arg_uuid, denied, ok, tool_error, AppToolContext};
use crate::authz::{Capability, Resource};
use crate::app_tools::AppTool;
use crate::repository::DbSenderKind;

pub(crate) fn tools(ctx: &Arc<AppToolContext>) -> Vec<AppTool> {
    vec![
        list_sessions(ctx),
        get_session(ctx),
        get_session_messages(ctx),
    ]
}

fn list_sessions(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("list_sessions")
        .description(
            "List the chat sessions in a project the user can access. Defaults to \
             the current project; pass project_id to inspect another project \
             you're a member of. Brief summary per session; `is_current` marks \
             the session this conversation is happening in.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string", "description": "Project UUID (optional; defaults to current project)" }
            }
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let project_id = arg_uuid(&args, "project_id").unwrap_or(ctx.project_id);
        // Project-scoped session read: capability is SessionRead, scoped to the
        // project so a non-current project is only listed for its members
        // (resolver returns false otherwise).
        if !ctx.authorize(Capability::SessionRead, Resource::Project(Some(project_id))).await {
            return denied("you cannot read sessions in that project");
        }
        let slug = ctx.project_slug(project_id).await;
        match ctx
            .repository
            .list_sessions_in_project(project_id, ctx.acting_user_id, None)
            .await
        {
            Ok(items) => ok(items
                .into_iter()
                .map(|s| json!({
                    "id": s.id,
                    "title": s.title,
                    "agent_type": s.agent_type,
                    "last_message_at": s.last_message_at,
                    "url": slug.as_deref().map(|sl| AppToolContext::session_link(sl, s.id)),
                    "is_current": s.id == ctx.session_id,
                }))
                .collect::<Vec<_>>()),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::SessionRead, desc, func)
}

fn get_session(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("get_session")
        .description(
            "Get a single chat session by id (metadata only, not its messages). \
             `is_current` marks the session this conversation is happening in.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string", "description": "Session UUID" }
            },
            "required": ["session_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "session_id") else {
            return tool_error("missing or invalid session_id");
        };
        if !ctx.authorize(Capability::SessionRead, Resource::Session(Some(id))).await {
            return denied("you cannot read this session");
        }
        match ctx.repository.get_session_with_authz(id, ctx.acting_user_id).await {
            Ok(Some((s, _access))) => {
                let url = ctx
                    .project_slug(s.project_id)
                    .await
                    .map(|slug| AppToolContext::session_link(&slug, s.id));
                ok(json!({
                    "id": s.id,
                    "title": s.title,
                    "agent_type": s.agent_type,
                    "model": s.model,
                    "origin": s.origin,
                    "created_at": s.created_at,
                    "last_message_at": s.last_message_at,
                    "url": url,
                    "is_current": s.id == ctx.session_id,
                }))
            }
            Ok(None) => tool_error("session not found"),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::SessionRead, desc, func)
}

fn get_session_messages(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("get_session_messages")
        .description(
            "Read the recent messages of a chat session (oldest→newest within the \
             returned window). The result's `is_current` tells you whether this is \
             the session you are currently in. Use to summarize or answer \
             questions about a past conversation.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string", "description": "Session UUID" },
                "limit": { "type": "integer", "description": "Max turns to return (1-200). Default 50." }
            },
            "required": ["session_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "session_id") else {
            return tool_error("missing or invalid session_id");
        };
        if !ctx.authorize(Capability::SessionRead, Resource::Session(Some(id))).await {
            return denied("you cannot read this session");
        }
        let limit = args
            .pointer("/limit")
            .and_then(|v| v.as_unsigned())
            .map(|n| (n as u32).clamp(1, 200))
            .unwrap_or(50);
        // Link the session itself (slug looked up via its project).
        let url = match ctx.repository.get_session_with_authz(id, ctx.acting_user_id).await {
            Ok(Some((s, _))) => ctx
                .project_slug(s.project_id)
                .await
                .map(|slug| AppToolContext::session_link(&slug, id)),
            _ => None,
        };
        match ctx.repository.get_messages_window(id, Some(limit), None).await {
            Ok(rows) => {
                let messages: Vec<serde_json::Value> = rows
                    .into_iter()
                    .map(|r| {
                        let text: String =
                            r.message.contents.iter().filter_map(|p| p.as_text()).collect();
                        let sender = match r.sender_kind {
                            DbSenderKind::User => "user",
                            DbSenderKind::Agent => "agent",
                        };
                        json!({
                            "seq": r.seq,
                            "sender": sender,
                            "sender_name": r.sender_name,
                            "created_at": r.created_at,
                            "text": text,
                        })
                    })
                    .collect();
                ok(json!({
                    "session_id": id,
                    "url": url,
                    "is_current": id == ctx.session_id,
                    "messages": messages,
                }))
            }
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::SessionRead, desc, func)
}
