//! Automation tools: inspect automations, triggers, and run history, and
//! create/update/run/delete automations for the projects the acting user can
//! access. Each call is gated by the two-layer policy.
//!
//! Outputs are trimmed to what an agent needs to reason and report: `list_*`
//! returns a brief summary per item; the `get_*` variant adds the detail.
//! Internal/storage fields (lease timestamps, webhook hashes, redundant ids)
//! are omitted.

use std::sync::Arc;

use ailoy::{to_value, tool::ToolDescBuilder, tool_func};
use chrono::Utc;
use serde_json::json;

use crate::app_tools::context::{
    arg_bool, arg_str, arg_strings, arg_uuid, denied, ok, tool_error, AppToolContext,
};
use crate::authz::{Capability, Resource};
use crate::app_tools::AppTool;
use crate::model::TriggerResponse;

pub(crate) fn tools(ctx: &Arc<AppToolContext>) -> Vec<AppTool> {
    vec![
        // read
        list_automations(ctx),
        get_automation(ctx),
        list_triggers(ctx),
        list_runs(ctx),
        get_run(ctx),
        list_run_events(ctx),
        // write / run
        create_automation(ctx),
        update_automation(ctx),
        create_trigger(ctx),
        run_automation(ctx),
        delete_automation(ctx),
    ]
}

/// Validate an agent_type string against the catalog (client error if unknown).
fn normalize_agent_type(s: &str) -> Result<String, String> {
    crate::model::AgentType::from_str(s)
        .map(|a| a.as_str().to_string())
        .ok_or_else(|| format!("unknown agent_type: {s}"))
}

/// Validate a model pin against the catalog (client error if unknown).
fn validate_model(model: &str) -> Result<(), String> {
    if crate::model::catalog_entry(model).is_none() {
        return Err(format!("unknown model: {model}"));
    }
    Ok(())
}

fn list_automations(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("list_automations")
        .description(
            "List the automations in a project. Defaults to the current project \
             (the one this chat belongs to); pass project_id to inspect another \
             project you're a member of. Brief summary per automation; call \
             get_automation for full configuration.",
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
        // Project-scoped automation read: capability is AutomationRead, scoped to
        // the project so a non-current project is only visible to its members
        // (resolver returns false otherwise).
        if !ctx.authorize(Capability::AutomationRead, Resource::Project(Some(project_id))).await {
            return denied("you cannot read automations in that project");
        }
        let slug = ctx.project_slug(project_id).await;
        match ctx.repository.list_automations_in_project(project_id).await {
            Ok(items) => ok(items
                .into_iter()
                .map(|a| json!({
                    "id": a.id,
                    "name": a.name,
                    "description": a.description,
                    "enabled": a.enabled,
                    "agent_type": a.agent_type,
                    "url": slug.as_deref().map(|sl| AppToolContext::automation_link(sl, a.id)),
                }))
                .collect::<Vec<_>>()),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationRead, desc, func)
}

fn get_automation(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("get_automation")
        .description("Get a single automation's full configuration by its id.")
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" }
            },
            "required": ["automation_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "automation_id") else {
            return tool_error("missing or invalid automation_id");
        };
        if !ctx.authorize(Capability::AutomationRead, Resource::Automation(Some(id))).await {
            return denied("you cannot read this automation");
        }
        match ctx.repository.get_automation(id).await {
            Ok(Some(a)) => {
                let url = ctx
                    .project_slug(a.project_id)
                    .await
                    .map(|slug| AppToolContext::automation_link(&slug, a.id));
                ok(json!({
                    "id": a.id,
                    "name": a.name,
                    "description": a.description,
                    "prompts": a.prompts,
                    "enabled": a.enabled,
                    "agent_type": a.agent_type,
                    "model": a.model,
                    "url": url,
                }))
            }
            Ok(None) => tool_error("automation not found"),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationRead, desc, func)
}

fn list_triggers(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("list_automation_triggers")
        .description(
            "List the triggers (cron schedules, webhooks) configured on an automation.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" }
            },
            "required": ["automation_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "automation_id") else {
            return tool_error("missing or invalid automation_id");
        };
        if !ctx.authorize(Capability::AutomationRead, Resource::Automation(Some(id))).await {
            return denied("you cannot read this automation");
        }
        match ctx.repository.list_triggers_for_automation(id).await {
            Ok(rows) => {
                let mut items = Vec::with_capacity(rows.len());
                for t in rows {
                    match TriggerResponse::from_db(t) {
                        Ok(r) => items.push(json!({
                            "id": r.id,
                            "kind": r.kind,
                            "enabled": r.enabled,
                            "spec": r.spec,
                            "next_fire_at": r.next_fire_at,
                        })),
                        Err(e) => return tool_error(format!("trigger decode: {e}")),
                    }
                }
                ok(items)
            }
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationRead, desc, func)
}

fn list_runs(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("list_automation_runs")
        .description(
            "List recent runs of an automation (most recent first): id, status, \
             and scheduled time. Call get_automation_run for detail.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" },
                "limit": { "type": "integer", "description": "Max runs to return (1-200). Default 50." }
            },
            "required": ["automation_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "automation_id") else {
            return tool_error("missing or invalid automation_id");
        };
        if !ctx.authorize(Capability::AutomationRunRead, Resource::Automation(Some(id))).await {
            return denied("you cannot read this automation's runs");
        }
        let limit = args
            .pointer("/limit")
            .and_then(|v| v.as_unsigned())
            .map(|n| (n as i64).clamp(1, 200))
            .unwrap_or(50);
        let slug = ctx.automation_slug(id).await;
        match ctx.repository.list_runs_for_automation(id, limit, 0).await {
            Ok(items) => ok(items
                .into_iter()
                .map(|r| json!({
                    "id": r.id,
                    "status": r.status,
                    "scheduled_for": r.scheduled_for,
                    "url": slug.as_deref().map(|sl| AppToolContext::run_link(sl, id, r.id)),
                }))
                .collect::<Vec<_>>()),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationRunRead, desc, func)
}

fn get_run(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("get_automation_run")
        .description(
            "Get a single automation run by id: status, timing, the agent/model \
             it used, and the session_id it produced (use get_session_messages \
             on that to see what the run did).",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" },
                "run_id": { "type": "string", "description": "Run UUID" }
            },
            "required": ["automation_id", "run_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let (Some(automation_id), Some(run_id)) =
            (arg_uuid(&args, "automation_id"), arg_uuid(&args, "run_id"))
        else {
            return tool_error("missing or invalid automation_id/run_id");
        };
        if !ctx.authorize(Capability::AutomationRunRead, Resource::Automation(Some(automation_id))).await {
            return denied("you cannot read this automation's runs");
        }
        match ctx.repository.get_run(run_id).await {
            Ok(Some(r)) if r.automation_id == automation_id => {
                let url = ctx
                    .automation_slug(automation_id)
                    .await
                    .map(|slug| AppToolContext::run_link(&slug, automation_id, r.id));
                ok(json!({
                    "id": r.id,
                    "status": r.status,
                    "scheduled_for": r.scheduled_for,
                    "started_at": r.created_at,
                    "updated_at": r.updated_at,
                    "agent_type": r.agent_type,
                    "model": r.model,
                    "session_id": r.session_id,
                    "url": url,
                }))
            }
            Ok(_) => tool_error("run not found"),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationRunRead, desc, func)
}

fn list_run_events(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("list_automation_run_events")
        .description(
            "List the lifecycle events of an automation run (queued, started, \
             succeeded, failed, …) for debugging why a run behaved as it did.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" },
                "run_id": { "type": "string", "description": "Run UUID" }
            },
            "required": ["automation_id", "run_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let (Some(automation_id), Some(run_id)) =
            (arg_uuid(&args, "automation_id"), arg_uuid(&args, "run_id"))
        else {
            return tool_error("missing or invalid automation_id/run_id");
        };
        if !ctx.authorize(Capability::AutomationRunRead, Resource::Automation(Some(automation_id))).await {
            return denied("you cannot read this automation's runs");
        }
        // Confirm the run belongs to the named automation before exposing events.
        match ctx.repository.get_run(run_id).await {
            Ok(Some(r)) if r.automation_id == automation_id => {}
            Ok(_) => return tool_error("run not found"),
            Err(e) => return tool_error(e),
        }
        match ctx.repository.list_events_for_run(run_id).await {
            Ok(items) => ok(items
                .into_iter()
                .map(|e| json!({
                    "ts": e.ts,
                    "kind": e.kind,
                    "payload": e.payload,
                }))
                .collect::<Vec<_>>()),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationRunRead, desc, func)
}

// ── write / run ──────────────────────────────────────────────────────────────

fn create_automation(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("create_automation")
        .description(
            "Create a new automation in the current project. `prompts` are the \
             instructions the automation's agent runs each time it fires. Returns \
             the created automation. (Add a schedule separately with \
             create_automation_trigger.)",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Automation name" },
                "description": { "type": "string", "description": "Optional description" },
                "prompts": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Instruction prompt(s) the automation runs"
                },
                "agent_type": { "type": "string", "description": "Optional agent surface (e.g. coworker)" },
                "model": { "type": "string", "description": "Optional model pin" }
            },
            "required": ["name"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        if !ctx.authorize(Capability::AutomationCreate, Resource::Automation(None)).await {
            return denied("you cannot create automations in this project");
        }
        // Loop guard: an automation must not create more automations (which,
        // with a schedule, would let it spawn recurring work and bypass the
        // run_automation guard).
        if ctx.in_automation_session().await {
            return denied("automation-run agents cannot create automations");
        }
        let Some(name) = arg_str(&args, "name") else {
            return tool_error("missing required field: name");
        };
        let description = arg_str(&args, "description");
        let prompts = arg_strings(&args, "prompts").unwrap_or_default();
        let agent_type = match arg_str(&args, "agent_type") {
            Some(s) => match normalize_agent_type(&s) {
                Ok(a) => Some(a),
                Err(e) => return tool_error(e),
            },
            None => None,
        };
        let model = arg_str(&args, "model");
        if let Some(ref m) = model
            && let Err(e) = validate_model(m)
        {
            return tool_error(e);
        }
        match ctx
            .repository
            .create_automation(ctx.project_id, name, description, prompts, agent_type, model, ctx.acting_user_id)
            .await
        {
            Ok(a) => ok(json!({
                "id": a.id,
                "name": a.name,
                "enabled": a.enabled,
                "agent_type": a.agent_type,
            })),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationCreate, desc, func)
}

fn update_automation(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("update_automation")
        .description(
            "Update an automation's name, description, prompts, or enabled state. \
             Omitted fields are left unchanged. Use enabled=false to pause it.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" },
                "name": { "type": "string" },
                "description": { "type": "string" },
                "prompts": { "type": "array", "items": { "type": "string" } },
                "enabled": { "type": "boolean", "description": "true to enable, false to pause" }
            },
            "required": ["automation_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "automation_id") else {
            return tool_error("missing or invalid automation_id");
        };
        if !ctx.authorize(Capability::AutomationUpdate, Resource::Automation(Some(id))).await {
            return denied("you cannot update this automation");
        }
        let name = arg_str(&args, "name");
        let description = arg_str(&args, "description").map(Some);
        let prompts = arg_strings(&args, "prompts");
        let enabled = arg_bool(&args, "enabled");
        match ctx
            .repository
            .update_automation(id, name, description, prompts, None, None, enabled)
            .await
        {
            Ok(a) => ok(json!({
                "id": a.id,
                "name": a.name,
                "enabled": a.enabled,
                "agent_type": a.agent_type,
            })),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationUpdate, desc, func)
}

fn create_trigger(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("create_automation_trigger")
        .description(
            "Add a cron schedule to an automation. `cron` is a standard 5-field \
             expression (e.g. '0 9 * * *' = every day 09:00). `tz` is an optional \
             IANA timezone (e.g. 'Asia/Seoul'); defaults to the server timezone.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" },
                "cron": { "type": "string", "description": "5-field cron expression" },
                "tz": { "type": "string", "description": "IANA timezone (optional)" }
            },
            "required": ["automation_id", "cron"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "automation_id") else {
            return tool_error("missing or invalid automation_id");
        };
        if !ctx.authorize(Capability::AutomationUpdate, Resource::Automation(Some(id))).await {
            return denied("you cannot modify this automation");
        }
        // Loop guard: adding a schedule from within an automation session would
        // let it spin up recurring runs, bypassing the run_automation guard.
        if ctx.in_automation_session().await {
            return denied("automation-run agents cannot add schedules");
        }
        let Some(expr) = arg_str(&args, "cron") else {
            return tool_error("missing required field: cron");
        };
        let tz = arg_str(&args, "tz");
        let default_tz = crate::cron::default_tz_name();
        let tz_name = tz.as_deref().unwrap_or(default_tz);
        let next = match crate::cron::next_fire_after(&expr, tz_name, Utc::now()) {
            Ok(n) => n,
            Err(e) => return tool_error(format!("invalid cron: {e}")),
        };
        let spec = crate::model::TriggerSpec::Cron { expr, tz };
        match ctx
            .repository
            .create_trigger_with_enabled(id, &spec, true, None, Some(next))
            .await
        {
            Ok(t) => ok(json!({
                "id": t.id,
                "kind": "cron",
                "enabled": t.enabled,
                "next_fire_at": next,
            })),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationUpdate, desc, func)
}

fn run_automation(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("run_automation")
        .description(
            "Trigger a one-off (manual) run of an automation now. Returns the \
             queued run id and the session it will run in. The run executes \
             asynchronously in the background.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" }
            },
            "required": ["automation_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "automation_id") else {
            return tool_error("missing or invalid automation_id");
        };
        if !ctx.authorize(Capability::AutomationRun, Resource::Automation(Some(id))).await {
            return denied("you cannot run this automation");
        }
        // Loop guard: an agent running inside an automation's own session must
        // not spawn further automation runs.
        if ctx.in_automation_session().await {
            return denied("automation-run agents cannot trigger more automation runs");
        }
        let automation = match ctx.repository.get_automation(id).await {
            Ok(Some(a)) => a,
            Ok(None) => return tool_error("automation not found"),
            Err(e) => return tool_error(e),
        };
        if !automation.enabled {
            return tool_error("automation is disabled; enable it before running");
        }
        let payload = json!({ "source": "manual", "actor_user_id": ctx.acting_user_id });
        match ctx
            .repository
            .create_automation_run_with_session(
                id,
                automation.project_id,
                ctx.acting_user_id,
                None,
                Utc::now(),
                None,
                Some(&payload),
                None,
            )
            .await
        {
            Ok(run) => ok(json!({
                "run_id": run.id,
                "session_id": run.session_id,
                "status": "queued",
            })),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationRun, desc, func)
}

fn delete_automation(ctx: &Arc<AppToolContext>) -> AppTool {
    let desc = ToolDescBuilder::new("delete_automation")
        .description(
            "Permanently delete an automation and its triggers/runs. Destructive — \
             confirm with the user first.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "automation_id": { "type": "string", "description": "Automation UUID" }
            },
            "required": ["automation_id"]
        }))
        .build();
    let ctx = ctx.clone();
    let func = tool_func!(async |args: Value| -> Value with [ctx = ctx.clone()] {
        let Some(id) = arg_uuid(&args, "automation_id") else {
            return tool_error("missing or invalid automation_id");
        };
        if !ctx.authorize(Capability::AutomationDelete, Resource::Automation(Some(id))).await {
            return denied("you cannot delete this automation");
        }
        match ctx.repository.delete_automation(id).await {
            Ok(true) => ok(json!({ "deleted": true, "id": id })),
            Ok(false) => tool_error("automation not found"),
            Err(e) => tool_error(e),
        }
    });
    AppTool::new(Capability::AutomationDelete, desc, func)
}
