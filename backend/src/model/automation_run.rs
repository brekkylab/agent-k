use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::DbAutomationRun;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Succeeded => "succeeded",
            RunStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(RunStatus::Queued),
            "running" => Some(RunStatus::Running),
            "succeeded" => Some(RunStatus::Succeeded),
            "failed" => Some(RunStatus::Failed),
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
