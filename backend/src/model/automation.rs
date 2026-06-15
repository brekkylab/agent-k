use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::{DbAutomation, DbAutomationRun, DbAutomationRunEvent, DbAutomationTrigger};

// ── automation ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct AutomationResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub prompts: Vec<String>,
    pub enabled: bool,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DbAutomation> for AutomationResponse {
    fn from(a: DbAutomation) -> Self {
        Self {
            id: a.id,
            project_id: a.project_id,
            name: a.name,
            description: a.description,
            prompts: a.prompts,
            enabled: a.enabled,
            agent_type: a.agent_type,
            model: a.model,
            created_by: a.created_by,
            created_at: a.created_at,
            updated_at: a.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAutomationRequest {
    /// Project UUID, active slug, or retired slug — backend resolves all three.
    pub project_ref: String,
    pub name: String,
    pub description: Option<String>,
    pub prompts: Vec<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAutomationRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub prompts: Option<Vec<String>>,
    pub enabled: Option<bool>,
    /// Agent surface for triggered runs. Absent = unchanged; a string sets it.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Model pin for triggered runs. Tri-state: absent = unchanged,
    /// `null` = recommended, string = pin a specific model.
    #[serde(default, deserialize_with = "double_option")]
    pub model: Option<Option<String>>,
}

/// Distinguish an absent JSON field from an explicit `null` for PATCH semantics
/// — without it, serde collapses both to `None` and a field can't be cleared.
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(de)?))
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AutomationListResponse {
    pub items: Vec<AutomationResponse>,
}

// ── trigger ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Cron,
    Webhook,
}

impl TriggerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerKind::Cron => "cron",
            TriggerKind::Webhook => "webhook",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "cron" => Some(TriggerKind::Cron),
            "webhook" => Some(TriggerKind::Webhook),
            _ => None,
        }
    }
}

/// API-shape: internally tagged by `kind`. DB-shape: `kind` column +
/// untagged variant fields in `spec_json` (see `to_db_spec_json` / `from_db`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerSpec {
    Cron {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
    Webhook {},
}

impl TriggerSpec {
    pub fn kind(&self) -> TriggerKind {
        match self {
            TriggerSpec::Cron { .. } => TriggerKind::Cron,
            TriggerSpec::Webhook { .. } => TriggerKind::Webhook,
        }
    }

    /// Serialize variant fields (without `kind`) for `spec_json` storage.
    pub fn to_db_spec_json(&self) -> serde_json::Result<String> {
        match self {
            TriggerSpec::Cron { expr, tz } => serde_json::to_string(&serde_json::json!({
                "expr": expr,
                "tz": tz,
            })),
            TriggerSpec::Webhook {} => serde_json::to_string(&serde_json::json!({})),
        }
    }

    /// Reconstruct from (kind, spec_json) pair as stored in the DB.
    pub fn from_db(kind: TriggerKind, spec_json: &str) -> serde_json::Result<Self> {
        match kind {
            TriggerKind::Cron => {
                #[derive(Deserialize)]
                struct CronFields {
                    expr: String,
                    #[serde(default)]
                    tz: Option<String>,
                }
                let CronFields { expr, tz } = serde_json::from_str(spec_json)?;
                Ok(TriggerSpec::Cron { expr, tz })
            }
            TriggerKind::Webhook => Ok(TriggerSpec::Webhook {}),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerResponse {
    pub id: Uuid,
    pub automation_id: Uuid,
    pub kind: TriggerKind,
    pub spec: TriggerSpec,
    pub enabled: bool,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TriggerResponse {
    pub fn from_db(t: DbAutomationTrigger) -> serde_json::Result<Self> {
        let spec = TriggerSpec::from_db(t.kind, &t.spec_json)?;
        Ok(Self {
            id: t.id,
            automation_id: t.automation_id,
            kind: t.kind,
            spec,
            enabled: t.enabled,
            next_fire_at: t.next_fire_at,
            created_at: t.created_at,
            updated_at: t.updated_at,
        })
    }
}

/// Trigger creation body is `TriggerSpec` directly (avoids serde flatten +
/// deny_unknown_fields conflict from a wrapper struct).
pub type CreateTriggerRequest = TriggerSpec;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateTriggerRequest {
    pub spec: Option<TriggerSpec>,
    pub enabled: Option<bool>,
}

/// Creation response. For webhook triggers, `webhook_token` is the only
/// chance to read the plaintext token; subsequent GETs never include it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedTriggerResponse {
    pub trigger: TriggerResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_token: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerListResponse {
    pub items: Vec<TriggerResponse>,
}

// ── occurrences (computed schedule preview) ──────────────────────────────────

/// A single upcoming scheduled fire, expanded from a cron trigger's expression.
/// Not persisted — computed on demand for the calendar view.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OccurrenceResponse {
    pub trigger_id: Uuid,
    pub automation_id: Uuid,
    pub automation_name: String,
    /// The exact fire instant (UTC). Clients localize for display.
    pub fire_at: DateTime<Utc>,
    /// The trigger's timezone (the cron expr is evaluated in this zone).
    pub tz: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OccurrenceListResponse {
    pub items: Vec<OccurrenceResponse>,
    /// `true` if any trigger's expansion hit the per-trigger cap within the
    /// window (the calendar is showing a partial set for at least one trigger).
    pub truncated: bool,
}

// ── run ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Succeeded => "succeeded",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(RunStatus::Queued),
            "running" => Some(RunStatus::Running),
            "succeeded" => Some(RunStatus::Succeeded),
            "failed" => Some(RunStatus::Failed),
            "cancelled" => Some(RunStatus::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunResponse {
    pub id: Uuid,
    pub automation_id: Uuid,
    pub trigger_id: Option<Uuid>,
    pub session_id: Uuid,
    pub status: RunStatus,
    pub scheduled_for: DateTime<Utc>,
    pub lease_until: Option<DateTime<Utc>>,
    pub previous_run_id: Option<Uuid>,
    /// Agent surface the run's session used; `None` if not joined/unknown.
    pub agent_type: Option<String>,
    /// Effective model the run's session used (materialized at build); `None`
    /// before the session agent is built.
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DbAutomationRun> for RunResponse {
    fn from(r: DbAutomationRun) -> Self {
        Self {
            id: r.id,
            automation_id: r.automation_id,
            trigger_id: r.trigger_id,
            session_id: r.session_id,
            status: r.status,
            scheduled_for: r.scheduled_for,
            lease_until: r.lease_until,
            previous_run_id: r.previous_run_id,
            agent_type: r.agent_type,
            model: r.model,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Request body for `POST /automations/:id/runs`. Currently no payload.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CreateRunRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunListResponse {
    pub items: Vec<RunResponse>,
}

// ── run event ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Triggered,
    Queued,
    Started,
    Succeeded,
    Failed,
    RetryScheduled,
    RetrySkipped,
    LeaseLost,
    StepStarted,
    StepFinished,
    Cancelled,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Triggered => "triggered",
            EventKind::Queued => "queued",
            EventKind::Started => "started",
            EventKind::Succeeded => "succeeded",
            EventKind::Failed => "failed",
            EventKind::RetryScheduled => "retry_scheduled",
            EventKind::RetrySkipped => "retry_skipped",
            EventKind::LeaseLost => "lease_lost",
            EventKind::StepStarted => "step_started",
            EventKind::StepFinished => "step_finished",
            EventKind::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "triggered" => Some(EventKind::Triggered),
            "queued" => Some(EventKind::Queued),
            "started" => Some(EventKind::Started),
            "succeeded" => Some(EventKind::Succeeded),
            "failed" => Some(EventKind::Failed),
            "retry_scheduled" => Some(EventKind::RetryScheduled),
            "retry_skipped" => Some(EventKind::RetrySkipped),
            "lease_lost" => Some(EventKind::LeaseLost),
            "step_started" => Some(EventKind::StepStarted),
            "step_finished" => Some(EventKind::StepFinished),
            "cancelled" => Some(EventKind::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EventResponse {
    pub id: i64,
    pub run_id: Uuid,
    pub ts: DateTime<Utc>,
    pub kind: EventKind,
    /// Shape depends on `kind`. See typed helpers below.
    pub payload: Option<serde_json::Value>,
}

impl From<DbAutomationRunEvent> for EventResponse {
    fn from(e: DbAutomationRunEvent) -> Self {
        Self {
            id: e.id,
            run_id: e.run_id,
            ts: e.ts,
            kind: e.kind,
            payload: e.payload,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EventListResponse {
    pub items: Vec<EventResponse>,
}
