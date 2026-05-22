use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::DbAutomationTrigger;

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
