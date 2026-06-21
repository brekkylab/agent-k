//! Read-only user tools: identify the acting user and resolve other users by
//! username/id (basic public profile only — no email, no admin listing).

use std::sync::Arc;

use ailoy::{to_value, tool::ToolDescBuilder, tool_func};
use serde_json::json;

use crate::app_tools::context::{arg_str, arg_uuid, denied, ok, tool_error, AppToolContext};
use crate::app_tools::policy::{Capability, Resource};
use crate::app_tools::AppTool;

pub(crate) fn tools(ctx: &Arc<AppToolContext>) -> Vec<AppTool> {
    vec![whoami(ctx), lookup_user(ctx)]
}

fn whoami(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("whoami")
        .description(
            "Return the profile of the user this agent is acting on behalf of \
             (the session owner). Use to know who you are helping.",
        )
        .parameters(to_value!({ "type": "object", "properties": {} }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |_args: Value| -> Value with [ctx = ctx.clone()] {
        let me = ctx.acting_user_id;
        if !ctx.authorize(Capability::UserReadSelf, Resource::User(Some(me))).await {
            return denied("cannot read your own profile");
        }
        match ctx.repository.get_user_by_id(me).await {
            Ok(Some(u)) => ok(json!({
                "id": u.id,
                "username": u.username,
                "display_name": u.display_name,
                "role": u.role,
                "preferred_language": u.preferred_language,
            })),
            Ok(None) => tool_error("user not found"),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::UserReadSelf, desc, func)
}

fn lookup_user(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("lookup_user")
        .description(
            "Look up a user's basic public profile (id, username, display name) by \
             username or id. Useful for resolving who someone is (e.g. a project \
             member). Provide either `username` or `user_id`.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "username": { "type": "string", "description": "Exact username" },
                "user_id": { "type": "string", "description": "User UUID" }
            }
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let by_id = arg_uuid(&args, "user_id");
        let by_name = arg_str(&args, "username");
        if by_id.is_none() && by_name.is_none() {
            return tool_error("provide either username or user_id");
        }
        if !ctx.authorize(Capability::UserLookup, Resource::User(by_id)).await {
            return denied("you cannot look up users");
        }
        let result = match by_id {
            Some(id) => ctx.repository.get_user_by_id(id).await,
            None => ctx.repository.get_user_by_username(by_name.as_deref().unwrap_or("")).await,
        };
        match result {
            Ok(Some(u)) => ok(json!({
                "id": u.id,
                "username": u.username,
                "display_name": u.display_name,
            })),
            Ok(None) => tool_error("user not found"),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::UserLookup, desc, func)
}
