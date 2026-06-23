//! Shared context and helpers for app-control tools.

use std::sync::Arc;

use ailoy::datatype::Value;
use uuid::Uuid;

use crate::app_tools::policy::AgentPolicy;
use crate::authz::{Capability, PermissionResolver, Resource};
use crate::repository::AppRepository;

/// Everything an app-control tool needs to authorize and execute, captured once
/// when the agent is built and shared (cheaply, via `Arc`) by every tool.
///
/// `acting_user_id` is the session's `creator_id` — "the user who owns this
/// agent". All authorization is judged against this identity, regardless of
/// whether the session was started by a person or by an automation.
pub struct AppToolContext {
    pub repository: AppRepository,
    pub resolver: Arc<dyn PermissionResolver>,
    pub agent_policy: AgentPolicy,
    pub acting_user_id: Uuid,
    pub project_id: Uuid,
    pub session_id: Uuid,
}

impl AppToolContext {
    /// The single gate every tool calls before touching the repository.
    ///
    /// The agent can only ever do the intersection of what it was granted
    /// (agent scope, Layer B) and what the user could do (user permission,
    /// Layer A). A mismatched `(cap, resource)` pair isn't accepted by any arm
    /// of the resolver's match, so it resolves to `false` on its own.
    pub async fn authorize(&self, cap: Capability, resource: Resource) -> bool {
        self.agent_policy.grants(cap)
            && self
                .resolver
                .user_can(self.acting_user_id, cap, &resource)
                .await
    }

    /// The active slug for `project_id`, used to build frontend links. `None`
    /// (link omitted) if the project can't be loaded.
    pub async fn project_slug(&self, project_id: Uuid) -> Option<String> {
        match self.repository.get_project(project_id).await {
            Ok(Some(p)) => Some(p.slug),
            _ => None,
        }
    }

    /// Relative frontend link to a project, given its slug.
    pub fn project_link(slug: &str) -> String {
        format!("/projects/{slug}")
    }

    /// Relative frontend link to a session within a project.
    pub fn session_link(slug: &str, session_id: Uuid) -> String {
        format!("/projects/{slug}/sessions/{session_id}")
    }

    /// Relative frontend link that opens the automation list and selects an
    /// automation (via the page's `#{id}` hash) — a view link, not the
    /// `/automation/{id}` edit route.
    pub fn automation_link(slug: &str, automation_id: Uuid) -> String {
        format!("/projects/{slug}/automation#{automation_id}")
    }

    /// Relative frontend link that opens the automation list and selects a
    /// specific run (via the `#{run_id}` hash; the page resolves the run to its
    /// automation). `automation_id` is unused in the URL but kept in the
    /// signature for call-site clarity.
    pub fn run_link(slug: &str, _automation_id: Uuid, run_id: Uuid) -> String {
        format!("/projects/{slug}/automation#{run_id}")
    }

    /// The active slug of the project that owns `automation_id`, for building
    /// run/automation links. `None` if it can't be resolved.
    pub async fn automation_slug(&self, automation_id: Uuid) -> Option<String> {
        match self.repository.get_automation(automation_id).await {
            Ok(Some(a)) => self.project_slug(a.project_id).await,
            _ => None,
        }
    }

    /// Whether this agent is itself running inside an automation's session.
    /// Used to block automation-spawning actions (run/create/schedule) so an
    /// automation can't recursively beget more automation work. Best-effort: a
    /// lookup failure is treated as "not an automation session".
    pub async fn in_automation_session(&self) -> bool {
        matches!(
            self.repository.get_session(self.session_id).await,
            Ok(Some(s)) if s.origin == crate::repository::SessionOrigin::Automation
        )
    }
}

/// Standard "permission denied" tool result. Tools return this (rather than
/// failing the run) so the model can relay the refusal to the user.
pub fn denied(detail: impl std::fmt::Display) -> Value {
    Value::from(serde_json::json!({
        "error": "permission_denied",
        "detail": detail.to_string(),
    }))
}

/// Standard error tool result for internal/repository failures.
pub fn tool_error(detail: impl std::fmt::Display) -> Value {
    Value::from(serde_json::json!({
        "error": "tool_error",
        "detail": detail.to_string(),
    }))
}

/// Convert any serializable payload into an ailoy [`Value`] for a tool result,
/// degrading to a [`tool_error`] if serialization somehow fails.
pub fn ok<T: serde::Serialize>(payload: T) -> Value {
    match serde_json::to_value(payload) {
        Ok(json) => Value::from(json),
        Err(e) => tool_error(format!("failed to serialize result: {e}")),
    }
}

/// Parse a UUID argument from tool input, accepting either a string field.
pub fn arg_uuid(args: &Value, key: &str) -> Option<Uuid> {
    args.pointer(&format!("/{key}"))
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s.trim()).ok())
}

/// Parse an optional string argument from tool input.
pub fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.pointer(&format!("/{key}"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Parse an optional boolean argument from tool input.
pub fn arg_bool(args: &Value, key: &str) -> Option<bool> {
    args.pointer(&format!("/{key}")).and_then(|v| v.as_bool())
}

/// Parse an optional array-of-strings argument (drops blanks).
pub fn arg_strings(args: &Value, key: &str) -> Option<Vec<String>> {
    args.pointer(&format!("/{key}"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
}
