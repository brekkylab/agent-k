//! Read-only project tools: list the user's projects and a project's members.
//! Outputs carry only what an agent needs to identify and reference them.

use std::sync::Arc;

use ailoy::{to_value, tool::ToolDescBuilder, tool_func};
use serde_json::json;

use crate::app_tools::context::{arg_str, arg_uuid, denied, ok, tool_error, AppToolContext};
use crate::authz::{Capability, Resource};
use crate::app_tools::AppTool;

pub(crate) fn tools(ctx: &Arc<AppToolContext>) -> Vec<AppTool> {
    vec![
        list_projects(ctx),
        list_members(ctx),
        add_member(ctx),
        remove_member(ctx),
    ]
}

fn list_projects(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("list_projects")
        .description("List the projects the user is a member of or owns (id, name, slug).")
        .parameters(to_value!({ "type": "object", "properties": {} }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |_args: Value| -> Value with [ctx = ctx.clone()] {
        if !ctx.authorize(Capability::ProjectRead, Resource::Project(None)).await {
            return denied("you cannot list projects");
        }
        match ctx.repository.list_projects_for_user(ctx.acting_user_id).await {
            Ok(items) => ok(items
                .into_iter()
                .map(|p| json!({
                    "id": p.id,
                    "name": p.name,
                    "slug": p.slug,
                    "description": p.description,
                    "url": AppToolContext::project_link(&p.slug),
                    "is_current": p.id == ctx.project_id,
                }))
                .collect::<Vec<_>>()),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::ProjectRead, desc, func)
}

fn list_members(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("list_project_members")
        .description(
            "List the members of a project (id, username, display name). Defaults \
             to the current project when project_id is omitted.",
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
        if !ctx.authorize(Capability::MemberRead, Resource::Project(Some(project_id))).await {
            return denied("you cannot read this project's members");
        }
        match ctx.repository.list_project_members(project_id).await {
            Ok(rows) => ok(rows
                .into_iter()
                .map(|(u, _added_at)| json!({
                    "user_id": u.id,
                    "username": u.username,
                    "display_name": u.display_name,
                }))
                .collect::<Vec<_>>()),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::MemberRead, desc, func)
}

fn add_member(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("add_project_member")
        .description(
            "Add a user (by username) to a project. Requires project ownership. \
             Defaults to the current project when project_id is omitted.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "username": { "type": "string", "description": "Exact username to add" },
                "project_id": { "type": "string", "description": "Project UUID (optional; defaults to current project)" }
            },
            "required": ["username"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let project_id = arg_uuid(&args, "project_id").unwrap_or(ctx.project_id);
        if !ctx.authorize(Capability::MemberManage, Resource::Project(Some(project_id))).await {
            return denied("you must be the project owner to manage members");
        }
        let Some(username) = arg_str(&args, "username") else {
            return tool_error("missing required field: username");
        };
        let target = match ctx.repository.get_user_by_username(&username).await {
            Ok(Some(u)) => u,
            Ok(None) => return tool_error(format!("user not found: {username}")),
            Err(e) => return tool_error(e),
        };
        match ctx.repository.add_project_member(project_id, target.id).await {
            Ok(()) => ok(json!({ "added": true, "user_id": target.id, "username": target.username })),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::MemberManage, desc, func)
}

fn remove_member(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("remove_project_member")
        .description(
            "Remove a member from a project (by username or user_id). Requires \
             project ownership. Destructive — confirm with the user first. \
             Defaults to the current project when project_id is omitted.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "username": { "type": "string", "description": "Exact username to remove" },
                "user_id": { "type": "string", "description": "User UUID to remove" },
                "project_id": { "type": "string", "description": "Project UUID (optional; defaults to current project)" }
            }
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let project_id = arg_uuid(&args, "project_id").unwrap_or(ctx.project_id);
        if !ctx.authorize(Capability::MemberManage, Resource::Project(Some(project_id))).await {
            return denied("you must be the project owner to manage members");
        }
        // Resolve the target user from either user_id or username.
        let target_id = match arg_uuid(&args, "user_id") {
            Some(id) => id,
            None => match arg_str(&args, "username") {
                Some(username) => match ctx.repository.get_user_by_username(&username).await {
                    Ok(Some(u)) => u.id,
                    Ok(None) => return tool_error(format!("user not found: {username}")),
                    Err(e) => return tool_error(e),
                },
                None => return tool_error("provide either username or user_id"),
            },
        };
        match ctx.repository.remove_project_member(project_id, target_id).await {
            Ok(true) => ok(json!({ "removed": true, "user_id": target_id })),
            Ok(false) => tool_error("member not found in project"),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::MemberManage, desc, func)
}
