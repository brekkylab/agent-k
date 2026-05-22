use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::DbAutomationRunEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Triggered,
    Queued,
    Started,
    Succeeded,
    Failed,
    RetryScheduled,
    LeaseLost,
    StepStarted,
    StepFinished,
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
            EventKind::LeaseLost => "lease_lost",
            EventKind::StepStarted => "step_started",
            EventKind::StepFinished => "step_finished",
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
            "lease_lost" => Some(EventKind::LeaseLost),
            "step_started" => Some(EventKind::StepStarted),
            "step_finished" => Some(EventKind::StepFinished),
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

