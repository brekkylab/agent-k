//! Automation persistence + lifecycle. Ported from backend-v1's
//! `repository/automation.rs`, adapted to v2 conventions (workspace-scoped,
//! `SqlitePool` store, `StateError`) and **single-prompt**: an automation runs
//! exactly one prompt, so there is no per-step cursor and the multi-step
//! machinery (StepStarted/StepFinished, resume) is gone. Cron is the only
//! trigger kind (webhook is out of scope).

use ailoy::agent::AgentSpec;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::{StateError, StateResult, parse_ts, parse_uuid};

/// Identity passed to agent-k's spec builders (mirrors router/session.rs).
const SESSION_AGENT_NAME: &str = "agent-k";

/// Total attempts including the first; RETRY_BACKOFFS has MAX_ATTEMPTS-1 gaps.
pub const MAX_ATTEMPTS: i64 = 3;

/// Reject an `agent_type` that names no known preset, so a bad value fails at
/// create/update rather than silently degrading to coworker at run time.
fn validate_agent_type(agent_type: &str) -> StateResult<()> {
    crate::model::AgentType::parse(agent_type)
        .map(|_| ())
        .ok_or_else(|| StateError::InvalidData(format!("unknown agent_type: {agent_type}")))
}

/// Build a session [`AgentSpec`] from an automation's stored agent surface.
/// Resolves the model through the agent-type catalog chain (the same path as
/// session creation), so an unpinned automation follows provider availability
/// instead of a hard-coded default. `agent_type` is validated at create/update;
/// an unknown value here still defaults to coworker.
pub fn build_spec(agent_type: &str, model: Option<&str>) -> AgentSpec {
    use agent_k::agents::{get_coworker_agent_spec, get_deep_research_agent_spec};
    use crate::model::{AgentType, resolve_model};

    let agent = AgentType::parse(agent_type).unwrap_or(AgentType::Coworker);
    let model = resolve_model(Some(agent.as_str()), model);
    match agent {
        AgentType::DeepResearch => get_deep_research_agent_spec(SESSION_AGENT_NAME, &model),
        AgentType::Coworker => get_coworker_agent_spec(SESSION_AGENT_NAME, &model, true),
    }
}

// ── enums ─────────────────────────────────────────────────────────────────

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

    pub fn parse(s: &str) -> Option<Self> {
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

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cron" => Some(TriggerKind::Cron),
            "webhook" => Some(TriggerKind::Webhook),
            _ => None,
        }
    }
}

/// API-shape: internally tagged by `kind`. DB-shape: `kind` column + untagged
/// variant fields in `spec_json`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerSpec {
    Cron {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
    /// Fired directly by an authenticated HTTP POST (automation-direct). The
    /// token isn't in the spec — its sha256 hash lives in the
    /// `webhook_token_hash` column and the raw token is shown once on create.
    Webhook {},
}

impl TriggerSpec {
    pub fn kind(&self) -> TriggerKind {
        match self {
            TriggerSpec::Cron { .. } => TriggerKind::Cron,
            TriggerSpec::Webhook {} => TriggerKind::Webhook,
        }
    }

    pub fn to_db_spec_json(&self) -> serde_json::Result<String> {
        match self {
            TriggerSpec::Cron { expr, tz } => {
                serde_json::to_string(&serde_json::json!({ "expr": expr, "tz": tz }))
            }
            TriggerSpec::Webhook {} => serde_json::to_string(&serde_json::json!({})),
        }
    }

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

/// sha256 hex of a webhook token — the lookup key stored in
/// `automation_triggers.webhook_token_hash`.
fn sha256_hex(s: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_ref());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 256 bits of entropy from two UUID v4s (OS-RNG backed).
fn generate_webhook_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunLogKind {
    Triggered,
    Queued,
    Started,
    Succeeded,
    Failed,
    RetryScheduled,
    RetrySkipped,
    LeaseLost,
    Cancelled,
}

impl RunLogKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunLogKind::Triggered => "triggered",
            RunLogKind::Queued => "queued",
            RunLogKind::Started => "started",
            RunLogKind::Succeeded => "succeeded",
            RunLogKind::Failed => "failed",
            RunLogKind::RetryScheduled => "retry_scheduled",
            RunLogKind::RetrySkipped => "retry_skipped",
            RunLogKind::LeaseLost => "lease_lost",
            RunLogKind::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "triggered" => Some(RunLogKind::Triggered),
            "queued" => Some(RunLogKind::Queued),
            "started" => Some(RunLogKind::Started),
            "succeeded" => Some(RunLogKind::Succeeded),
            "failed" => Some(RunLogKind::Failed),
            "retry_scheduled" => Some(RunLogKind::RetryScheduled),
            "retry_skipped" => Some(RunLogKind::RetrySkipped),
            "lease_lost" => Some(RunLogKind::LeaseLost),
            "cancelled" => Some(RunLogKind::Cancelled),
            _ => None,
        }
    }
}

// ── row structs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Automation {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub prompt: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub enabled: bool,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Automation {
    fn from_row(row: &SqliteRow) -> StateResult<Self> {
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "automations.id")?,
            workspace_id: parse_uuid(
                row.get::<String, _>("workspace_id"),
                "automations.workspace_id",
            )?,
            name: row.get("name"),
            description: row.get("description"),
            prompt: row.get("prompt"),
            agent_type: row.get("agent_type"),
            model: row.get("model"),
            enabled: row.get("enabled"),
            created_by: parse_uuid(row.get::<String, _>("created_by"), "automations.created_by")?,
            created_at: parse_ts(&row.get::<String, _>("created_at"), "automations.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "automations.updated_at")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AutomationTrigger {
    pub id: Uuid,
    pub automation_id: Uuid,
    pub kind: TriggerKind,
    pub spec_json: String,
    pub enabled: bool,
    pub next_fire_at: Option<DateTime<Utc>>,
    /// sha256 hex of the issued webhook token (webhook triggers only; else `None`).
    pub webhook_token_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AutomationTrigger {
    fn from_row(row: &SqliteRow) -> StateResult<Self> {
        let kind_s: String = row.get("kind");
        let kind = TriggerKind::parse(&kind_s)
            .ok_or_else(|| StateError::InvalidData(format!("invalid trigger kind '{kind_s}'")))?;
        let next_fire_at = row
            .get::<Option<String>, _>("next_fire_at")
            .map(|s| parse_ts(&s, "automation_triggers.next_fire_at"))
            .transpose()?;
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "automation_triggers.id")?,
            automation_id: parse_uuid(
                row.get::<String, _>("automation_id"),
                "automation_triggers.automation_id",
            )?,
            kind,
            spec_json: row.get("spec_json"),
            enabled: row.get("enabled"),
            next_fire_at,
            webhook_token_hash: row.get("webhook_token_hash"),
            created_at: parse_ts(
                &row.get::<String, _>("created_at"),
                "automation_triggers.created_at",
            )?,
            updated_at: parse_ts(
                &row.get::<String, _>("updated_at"),
                "automation_triggers.updated_at",
            )?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AutomationRun {
    pub id: Uuid,
    /// `None` for ad-hoc one-time runs (no owning automation).
    pub automation_id: Option<Uuid>,
    pub trigger_id: Option<Uuid>,
    pub session_id: Uuid,
    pub workspace_id: Uuid,
    /// Rendered prompt actually executed (snapshot).
    pub prompt: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub status: RunStatus,
    pub scheduled_for: DateTime<Utc>,
    pub lease_until: Option<DateTime<Utc>>,
    pub previous_run_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AutomationRun {
    fn from_row(row: &SqliteRow) -> StateResult<Self> {
        let status_s: String = row.get("status");
        let status = RunStatus::parse(&status_s)
            .ok_or_else(|| StateError::InvalidData(format!("invalid run status '{status_s}'")))?;
        let opt_uuid = |col: &str| -> StateResult<Option<Uuid>> {
            row.get::<Option<String>, _>(col)
                .map(|s| parse_uuid(s, "automation_runs"))
                .transpose()
        };
        let lease_until = row
            .get::<Option<String>, _>("lease_until")
            .map(|s| parse_ts(&s, "automation_runs.lease_until"))
            .transpose()?;
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "automation_runs.id")?,
            automation_id: opt_uuid("automation_id")?,
            trigger_id: opt_uuid("trigger_id")?,
            session_id: parse_uuid(row.get::<String, _>("session_id"), "automation_runs.session_id")?,
            workspace_id: parse_uuid(
                row.get::<String, _>("workspace_id"),
                "automation_runs.workspace_id",
            )?,
            prompt: row.get("prompt"),
            agent_type: row.get("agent_type"),
            model: row.get("model"),
            status,
            scheduled_for: parse_ts(
                &row.get::<String, _>("scheduled_for"),
                "automation_runs.scheduled_for",
            )?,
            lease_until,
            previous_run_id: opt_uuid("previous_run_id")?,
            created_at: parse_ts(
                &row.get::<String, _>("created_at"),
                "automation_runs.created_at",
            )?,
            updated_at: parse_ts(
                &row.get::<String, _>("updated_at"),
                "automation_runs.updated_at",
            )?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RunLog {
    pub id: i64,
    pub run_id: Uuid,
    pub ts: DateTime<Utc>,
    pub kind: RunLogKind,
    pub payload: Option<serde_json::Value>,
}

impl RunLog {
    fn from_row(row: &SqliteRow) -> StateResult<Self> {
        let kind_s: String = row.get("kind");
        let kind = RunLogKind::parse(&kind_s)
            .ok_or_else(|| StateError::InvalidData(format!("invalid event kind '{kind_s}'")))?;
        let payload = row
            .get::<Option<String>, _>("payload")
            .map(|s| serde_json::from_str(&s))
            .transpose()?;
        Ok(Self {
            id: row.get("id"),
            run_id: parse_uuid(row.get::<String, _>("run_id"), "automation_run_logs.run_id")?,
            ts: parse_ts(&row.get::<String, _>("ts"), "automation_run_logs.ts")?,
            kind,
            payload,
        })
    }
}

const AUTOMATION_COLS: &str =
    "id, workspace_id, name, description, prompt, agent_type, model, enabled, created_by, created_at, updated_at";
const TRIGGER_COLS: &str =
    "id, automation_id, kind, spec_json, enabled, next_fire_at, webhook_token_hash, created_at, updated_at";
const RUN_COLS: &str =
    "id, automation_id, trigger_id, session_id, workspace_id, prompt, agent_type, model, status, scheduled_for, lease_until, previous_run_id, created_at, updated_at";

/// Everything needed to enqueue a run + its session. `automation_id = None` for
/// ad-hoc one-time runs that depend on no automation.
struct NewRun {
    automation_id: Option<Uuid>,
    trigger_id: Option<Uuid>,
    workspace_id: Uuid,
    title: String,
    prompt: String,
    agent_type: String,
    model: Option<String>,
    previous_run_id: Option<Uuid>,
}

impl NewRun {
    /// Snapshot an automation's prompt + agent surface into a new run.
    fn from_automation(
        a: &Automation,
        trigger_id: Option<Uuid>,
        previous_run_id: Option<Uuid>,
    ) -> Self {
        Self {
            automation_id: Some(a.id),
            trigger_id,
            workspace_id: a.workspace_id,
            title: a.name.clone(),
            prompt: a.prompt.clone(),
            agent_type: a.agent_type.clone(),
            model: a.model.clone(),
            previous_run_id,
        }
    }

    /// Re-run the same snapshot as a prior run (for retries). Preserves the
    /// executed prompt rather than re-reading a possibly-changed template.
    fn retry_of(prev: &AutomationRun) -> Self {
        Self {
            automation_id: prev.automation_id,
            trigger_id: prev.trigger_id,
            workspace_id: prev.workspace_id,
            title: prev.prompt.chars().take(60).collect(),
            prompt: prev.prompt.clone(),
            agent_type: prev.agent_type.clone(),
            model: prev.model.clone(),
            previous_run_id: Some(prev.id),
        }
    }
}

// ── store ─────────────────────────────────────────────────────────────────

pub struct AutomationsState {
    db: SqlitePool,
}

impl AutomationsState {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    fn now() -> String {
        Utc::now().to_rfc3339()
    }

    // ── automations ─────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub async fn create_automation(
        &self,
        workspace_id: Uuid,
        name: String,
        description: Option<String>,
        prompt: String,
        agent_type: String,
        model: Option<String>,
        created_by: Uuid,
    ) -> StateResult<Automation> {
        validate_agent_type(&agent_type)?;
        let id = Uuid::new_v4();
        let now = Self::now();
        sqlx::query(
            "INSERT INTO automations (id, workspace_id, name, description, prompt, agent_type, model, enabled, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(&name)
        .bind(&description)
        .bind(&prompt)
        .bind(&agent_type)
        .bind(&model)
        .bind(created_by.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.db)
        .await?;

        Ok(Automation {
            id,
            workspace_id,
            name,
            description,
            prompt,
            agent_type,
            model,
            enabled: true,
            created_by,
            created_at: parse_ts(&now, "automations.created_at")?,
            updated_at: parse_ts(&now, "automations.updated_at")?,
        })
    }

    pub async fn get_automation(&self, id: Uuid) -> StateResult<Option<Automation>> {
        let sql = format!("SELECT {AUTOMATION_COLS} FROM automations WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id.to_string())
            .fetch_optional(&self.db)
            .await?;
        row.as_ref().map(Automation::from_row).transpose()
    }

    pub async fn list_automations_by_workspace(
        &self,
        workspace_id: Uuid,
    ) -> StateResult<Vec<Automation>> {
        let sql = format!(
            "SELECT {AUTOMATION_COLS} FROM automations WHERE workspace_id = ? ORDER BY created_at DESC"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(workspace_id.to_string())
            .fetch_all(&self.db)
            .await?;
        rows.iter().map(Automation::from_row).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_automation(
        &self,
        id: Uuid,
        name: Option<String>,
        description: Option<Option<String>>,
        prompt: Option<String>,
        agent_type: Option<String>,
        model: Option<Option<String>>,
        enabled: Option<bool>,
    ) -> StateResult<Automation> {
        let current = self.get_automation(id).await?.ok_or(StateError::NotFound)?;
        let name = name.unwrap_or(current.name);
        let description = description.unwrap_or(current.description);
        let prompt = prompt.unwrap_or(current.prompt);
        let agent_type = agent_type.unwrap_or(current.agent_type);
        validate_agent_type(&agent_type)?;
        let model = model.unwrap_or(current.model);
        let enabled = enabled.unwrap_or(current.enabled);
        let now = Self::now();

        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE automations SET name = ?, description = ?, prompt = ?, agent_type = ?, model = ?, enabled = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&description)
        .bind(&prompt)
        .bind(&agent_type)
        .bind(&model)
        .bind(enabled)
        .bind(&now)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;

        // Disabling cancels queued runs so nothing new starts under the old config.
        if current.enabled && !enabled {
            sqlx::query(
                "UPDATE automation_runs SET status = 'cancelled', lease_until = NULL, updated_at = ? \
                 WHERE automation_id = ? AND status = 'queued'",
            )
            .bind(&now)
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        Ok(Automation {
            id,
            workspace_id: current.workspace_id,
            name,
            description,
            prompt,
            agent_type,
            model,
            enabled,
            created_by: current.created_by,
            created_at: current.created_at,
            updated_at: parse_ts(&now, "automations.updated_at")?,
        })
    }

    /// Delete the automation. Its triggers cascade away, but its runs are
    /// preserved as an audit record — `automation_runs.automation_id` is
    /// `ON DELETE SET NULL`, and each run already snapshots the prompt + agent
    /// surface it executed, so the run + its events + session stay intact and
    /// self-describing. Returns `false` if no automation row existed.
    pub async fn delete_automation(&self, id: Uuid) -> StateResult<bool> {
        let res = sqlx::query("DELETE FROM automations WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        Ok(res.rows_affected() == 1)
    }

    // ── triggers ────────────────────────────────────────────────────────

    /// Create a trigger; for cron, `next_fire_at` is computed from the expr.
    pub async fn create_trigger(
        &self,
        automation_id: Uuid,
        spec: &TriggerSpec,
        enabled: bool,
        next_fire_at: Option<DateTime<Utc>>,
        webhook_token_hash: Option<String>,
    ) -> StateResult<AutomationTrigger> {
        let id = Uuid::new_v4();
        let now = Self::now();
        let spec_json = spec.to_db_spec_json()?;
        let next_s = next_fire_at.map(|t| t.to_rfc3339());
        sqlx::query(
            "INSERT INTO automation_triggers (id, automation_id, kind, spec_json, enabled, next_fire_at, webhook_token_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(automation_id.to_string())
        .bind(spec.kind().as_str())
        .bind(&spec_json)
        .bind(enabled)
        .bind(&next_s)
        .bind(&webhook_token_hash)
        .bind(&now)
        .bind(&now)
        .execute(&self.db)
        .await?;

        Ok(AutomationTrigger {
            id,
            automation_id,
            kind: spec.kind(),
            spec_json,
            enabled,
            next_fire_at,
            webhook_token_hash,
            created_at: parse_ts(&now, "automation_triggers.created_at")?,
            updated_at: parse_ts(&now, "automation_triggers.updated_at")?,
        })
    }

    pub async fn get_trigger(&self, id: Uuid) -> StateResult<Option<AutomationTrigger>> {
        let sql = format!("SELECT {TRIGGER_COLS} FROM automation_triggers WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id.to_string())
            .fetch_optional(&self.db)
            .await?;
        row.as_ref().map(AutomationTrigger::from_row).transpose()
    }

    pub async fn list_triggers(
        &self,
        automation_id: Uuid,
    ) -> StateResult<Vec<AutomationTrigger>> {
        let sql = format!(
            "SELECT {TRIGGER_COLS} FROM automation_triggers WHERE automation_id = ? ORDER BY created_at ASC"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(automation_id.to_string())
            .fetch_all(&self.db)
            .await?;
        rows.iter().map(AutomationTrigger::from_row).collect()
    }

    pub async fn delete_trigger(&self, id: Uuid) -> StateResult<bool> {
        let res = sqlx::query("DELETE FROM automation_triggers WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        Ok(res.rows_affected() == 1)
    }

    /// The enabled webhook trigger of `automation_id` whose secret matches
    /// `token`, or `None`. Used by the public webhook receive endpoint.
    /// The webhook trigger whose token hashes to `token_hash`, or `None`. The
    /// hash is globally unique, so the token alone identifies its trigger.
    pub async fn find_trigger_by_webhook_token_hash(
        &self,
        token_hash: &str,
    ) -> StateResult<Option<AutomationTrigger>> {
        let sql = format!(
            "SELECT {TRIGGER_COLS} FROM automation_triggers WHERE webhook_token_hash = ?"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(token_hash)
            .fetch_optional(&self.db)
            .await?;
        row.as_ref().map(AutomationTrigger::from_row).transpose()
    }

    /// sha256 hex of a webhook token — for the caller to hash a presented token
    /// before looking it up.
    pub fn webhook_token_hash(token: &str) -> String {
        sha256_hex(token)
    }

    /// Generate a fresh webhook token; returns `(token, hash)`. Store the hash,
    /// return the token to the client once.
    pub fn new_webhook_token() -> (String, String) {
        let token = generate_webhook_token();
        let hash = sha256_hex(&token);
        (token, hash)
    }

    /// Create a queued run fired directly by a webhook (automation-direct). The
    /// automation's stored prompt runs as-is — the POST body is not yet used as
    /// render input (that arrives with the event infrastructure). `None` if the
    /// automation is disabled.
    pub async fn create_webhook_run(
        &self,
        automation: &Automation,
        trigger_id: Uuid,
    ) -> StateResult<Option<AutomationRun>> {
        let spec = NewRun::from_automation(automation, Some(trigger_id), None);
        let payload = serde_json::json!({ "source": "webhook" });
        self.insert_run_with_session(spec, Utc::now(), Some(&payload)).await
    }

    /// Cron triggers whose `next_fire_at` has elapsed (enabled only).
    pub async fn list_due_cron_triggers(
        &self,
        now: DateTime<Utc>,
    ) -> StateResult<Vec<AutomationTrigger>> {
        let sql = format!(
            "SELECT {TRIGGER_COLS} FROM automation_triggers \
             WHERE kind = 'cron' AND enabled = 1 AND next_fire_at IS NOT NULL AND next_fire_at <= ? \
               AND automation_id IN (SELECT id FROM automations WHERE enabled = 1) \
             ORDER BY next_fire_at ASC"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(now.to_rfc3339())
            .fetch_all(&self.db)
            .await?;
        rows.iter().map(AutomationTrigger::from_row).collect()
    }

    // ── runs ────────────────────────────────────────────────────────────

    /// Enqueue a run + its session, emitting `triggered` + `queued` atomically.
    /// When `spec.automation_id` is set, the insert is gated on that automation
    /// still being enabled (returns `None` if disabled); ad-hoc runs
    /// (`automation_id = None`) are never gated.
    async fn insert_run_with_session(
        &self,
        spec: NewRun,
        scheduled_for: DateTime<Utc>,
        triggered_payload: Option<&serde_json::Value>,
    ) -> StateResult<Option<AutomationRun>> {
        let mut tx = self.db.begin().await?;
        match self
            .insert_run_in(&mut tx, spec, scheduled_for, triggered_payload)
            .await?
        {
            Some(run) => {
                tx.commit().await?;
                Ok(Some(run))
            }
            // Disabled: roll back to undo the session insert too.
            None => {
                tx.rollback().await?;
                Ok(None)
            }
        }
    }

    /// Insert the session + queued run (+ triggered/queued logs) on a caller
    /// transaction, so a cron fire can advance `next_fire_at` in the same tx.
    /// `Ok(None)` = the automation is disabled and no run was created; the caller
    /// rolls the tx back, undoing the session insert (and any advance it carries).
    async fn insert_run_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        spec: NewRun,
        scheduled_for: DateTime<Utc>,
        triggered_payload: Option<&serde_json::Value>,
    ) -> StateResult<Option<AutomationRun>> {
        let now = Self::now();
        let scheduled_s = scheduled_for.to_rfc3339();
        let agent_spec = build_spec(&spec.agent_type, spec.model.as_deref());
        let spec_json = serde_json::to_string(&agent_spec)?;
        let session_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();

        // Session for this run (no runenv/sandbox — automation runs are simple).
        // origin='automation' marks it worker-driven: the message API rejects
        // user turns on it, and it can be filtered out of the session list.
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, agent_id, title, spec, runenv, origin, created_at, updated_at) \
             VALUES (?, ?, NULL, ?, ?, 0, 'automation', ?, ?)",
        )
        .bind(session_id.to_string())
        .bind(spec.workspace_id.to_string())
        .bind(&spec.title)
        .bind(&spec_json)
        .bind(&now)
        .bind(&now)
        .execute(&mut **tx)
        .await?;

        // Run row. Automation-linked runs are gated on the automation still being
        // enabled (atomic against a concurrent disable); ad-hoc runs insert plainly.
        let run_cols = "INSERT INTO automation_runs \
             (id, automation_id, trigger_id, session_id, workspace_id, prompt, agent_type, model, status, scheduled_for, lease_until, previous_run_id, created_at, updated_at)";
        let res = if let Some(aid) = spec.automation_id {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "{run_cols} SELECT ?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, NULL, ?, ?, ? \
                  WHERE EXISTS (SELECT 1 FROM automations WHERE id = ? AND enabled = 1)"
            )))
            .bind(run_id.to_string())
            .bind(aid.to_string())
            .bind(spec.trigger_id.map(|u| u.to_string()))
            .bind(session_id.to_string())
            .bind(spec.workspace_id.to_string())
            .bind(&spec.prompt)
            .bind(&spec.agent_type)
            .bind(&spec.model)
            .bind(&scheduled_s)
            .bind(spec.previous_run_id.map(|u| u.to_string()))
            .bind(&now)
            .bind(&now)
            .bind(aid.to_string())
            .execute(&mut **tx)
            .await?
        } else {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "{run_cols} VALUES (?, NULL, ?, ?, ?, ?, ?, ?, 'queued', ?, NULL, ?, ?, ?)"
            )))
            .bind(run_id.to_string())
            .bind(spec.trigger_id.map(|u| u.to_string()))
            .bind(session_id.to_string())
            .bind(spec.workspace_id.to_string())
            .bind(&spec.prompt)
            .bind(&spec.agent_type)
            .bind(&spec.model)
            .bind(&scheduled_s)
            .bind(spec.previous_run_id.map(|u| u.to_string()))
            .bind(&now)
            .bind(&now)
            .execute(&mut **tx)
            .await?
        };

        if res.rows_affected() == 0 {
            // Automation disabled: no run. The caller rolls the tx back, which also
            // undoes the session insert above (and any next_fire_at advance).
            return Ok(None);
        }

        Self::insert_log(tx, run_id, RunLogKind::Triggered, triggered_payload, &now).await?;
        Self::insert_log(tx, run_id, RunLogKind::Queued, None, &now).await?;

        Ok(Some(AutomationRun {
            id: run_id,
            automation_id: spec.automation_id,
            trigger_id: spec.trigger_id,
            session_id,
            workspace_id: spec.workspace_id,
            prompt: spec.prompt,
            agent_type: spec.agent_type,
            model: spec.model,
            status: RunStatus::Queued,
            scheduled_for,
            lease_until: None,
            previous_run_id: spec.previous_run_id,
            created_at: parse_ts(&now, "automation_runs.created_at")?,
            updated_at: parse_ts(&now, "automation_runs.updated_at")?,
        }))
    }

    /// A run of `automation` scheduled for `at` (snapshotting its prompt + agent
    /// surface). `at = now` for an immediate manual run. Errors if disabled.
    pub async fn create_scheduled_run(
        &self,
        automation: &Automation,
        at: DateTime<Utc>,
        source: &str,
    ) -> StateResult<AutomationRun> {
        let payload = serde_json::json!({ "source": source, "scheduled_for": at.to_rfc3339() });
        self.insert_run_with_session(NewRun::from_automation(automation, None, None), at, Some(&payload))
            .await?
            .ok_or_else(|| StateError::InvalidData("automation is disabled".into()))
    }

    /// Ad-hoc one-time run: no automation, the run carries its own (rendered)
    /// prompt and agent surface. `at = now` runs immediately; a future `at`
    /// defers pickup until then. Never gated (there is no automation to disable).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_adhoc_run(
        &self,
        workspace_id: Uuid,
        title: String,
        prompt: String,
        agent_type: String,
        model: Option<String>,
        at: DateTime<Utc>,
        scheduled_by: Uuid,
    ) -> StateResult<AutomationRun> {
        let payload = serde_json::json!({
            "source": "one_time",
            "scheduled_by": scheduled_by.to_string(),
            "scheduled_for": at.to_rfc3339(),
        });
        let spec = NewRun {
            automation_id: None,
            trigger_id: None,
            workspace_id,
            title,
            prompt,
            agent_type,
            model,
            previous_run_id: None,
        };
        self.insert_run_with_session(spec, at, Some(&payload))
            .await?
            .ok_or_else(|| StateError::InvalidData("failed to create run".into()))
    }

    /// Cron fire: advance `next_fire_at` and insert the run in one tx, so a crash
    /// rolls back both and the next tick retries cleanly (no missed run, no
    /// duplicate). A disabled automation (a rare disable-vs-fire race — the due
    /// scan already excludes disabled ones) rolls back: the tick never happened.
    /// `Ok(None)` = the automation is disabled.
    pub async fn fire_cron_trigger(
        &self,
        automation: &Automation,
        trigger_id: Uuid,
        scheduled_for: DateTime<Utc>,
        next_fire_at: DateTime<Utc>,
        triggered_payload: &serde_json::Value,
    ) -> StateResult<Option<AutomationRun>> {
        let mut tx = self.db.begin().await?;
        sqlx::query("UPDATE automation_triggers SET next_fire_at = ?, updated_at = ? WHERE id = ?")
            .bind(next_fire_at.to_rfc3339())
            .bind(Self::now())
            .bind(trigger_id.to_string())
            .execute(&mut *tx)
            .await?;
        match self
            .insert_run_in(
                &mut tx,
                NewRun::from_automation(automation, Some(trigger_id), None),
                scheduled_for,
                Some(triggered_payload),
            )
            .await?
        {
            Some(run) => {
                tx.commit().await?;
                Ok(Some(run))
            }
            None => {
                tx.rollback().await?;
                Ok(None)
            }
        }
    }

    pub async fn get_run(&self, id: Uuid) -> StateResult<Option<AutomationRun>> {
        let sql = format!("SELECT {RUN_COLS} FROM automation_runs WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id.to_string())
            .fetch_optional(&self.db)
            .await?;
        row.as_ref().map(AutomationRun::from_row).transpose()
    }

    pub async fn list_runs(&self, automation_id: Uuid) -> StateResult<Vec<AutomationRun>> {
        let sql = format!(
            "SELECT {RUN_COLS} FROM automation_runs WHERE automation_id = ? ORDER BY created_at DESC"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(automation_id.to_string())
            .fetch_all(&self.db)
            .await?;
        rows.iter().map(AutomationRun::from_row).collect()
    }

    pub async fn list_runs_by_workspace(&self, workspace_id: Uuid) -> StateResult<Vec<AutomationRun>> {
        let sql = format!(
            "SELECT {RUN_COLS} FROM automation_runs WHERE workspace_id = ? ORDER BY created_at DESC"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(workspace_id.to_string())
            .fetch_all(&self.db)
            .await?;
        rows.iter().map(AutomationRun::from_row).collect()
    }

    /// Atomically claim the oldest due `queued` run: flip to `running` with a
    /// lease. Returns the claimed run, or `None` if nothing is due.
    pub async fn claim_due_run(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> StateResult<Option<AutomationRun>> {
        let now_s = now.to_rfc3339();
        let mut tx = self.db.begin().await?;
        let sql = format!(
            "SELECT {RUN_COLS} FROM automation_runs \
             WHERE status = 'queued' AND scheduled_for <= ? ORDER BY scheduled_for ASC LIMIT 1"
        );
        let Some(row) = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&now_s)
            .fetch_optional(&mut *tx)
            .await?
        else {
            tx.rollback().await?;
            return Ok(None);
        };
        let run = AutomationRun::from_row(&row)?;
        let res = sqlx::query(
            "UPDATE automation_runs SET status = 'running', lease_until = ?, updated_at = ? \
             WHERE id = ? AND status = 'queued'",
        )
        .bind(lease_until.to_rfc3339())
        .bind(&now_s)
        .bind(run.id.to_string())
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() != 1 {
            // Lost the race to another worker; treat as nothing claimed.
            tx.rollback().await?;
            return Ok(None);
        }
        tx.commit().await?;
        Ok(Some(AutomationRun {
            status: RunStatus::Running,
            lease_until: Some(lease_until),
            ..run
        }))
    }

    /// Renew the lease on an owned `running` run. `false` if it is no longer ours.
    pub async fn renew_lease(&self, run_id: Uuid, new_lease: DateTime<Utc>) -> StateResult<bool> {
        let res = sqlx::query(
            "UPDATE automation_runs SET lease_until = ?, updated_at = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(new_lease.to_rfc3339())
        .bind(Self::now())
        .bind(run_id.to_string())
        .execute(&self.db)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Atomically write the terminal event + status (clearing the lease). The
    /// UPDATE is guarded by `status='running'`; `Ok(false)` if reaped from under us.
    pub async fn finalize_run(
        &self,
        run_id: Uuid,
        status: RunStatus,
        event_kind: RunLogKind,
        event_payload: Option<&serde_json::Value>,
    ) -> StateResult<bool> {
        let now = Self::now();
        let mut tx = self.db.begin().await?;
        let res = sqlx::query(
            "UPDATE automation_runs SET status = ?, lease_until = NULL, updated_at = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(status.as_str())
        .bind(&now)
        .bind(run_id.to_string())
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(false);
        }
        Self::insert_log(&mut tx, run_id, event_kind, event_payload, &now).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Cancel a queued/running run + emit `cancelled`. `false` if already terminal.
    pub async fn cancel_run(
        &self,
        run_id: Uuid,
        payload: &serde_json::Value,
    ) -> StateResult<bool> {
        let now = Self::now();
        let mut tx = self.db.begin().await?;
        let res = sqlx::query(
            "UPDATE automation_runs SET status = 'cancelled', lease_until = NULL, updated_at = ? \
             WHERE id = ? AND status IN ('queued', 'running')",
        )
        .bind(&now)
        .bind(run_id.to_string())
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        Self::insert_log(&mut tx, run_id, RunLogKind::Cancelled, Some(payload), &now).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Attempt number of `run_id` = depth of the `previous_run_id` chain (1-based).
    pub async fn compute_run_attempt(&self, run_id: Uuid) -> StateResult<i64> {
        let row = sqlx::query(
            "WITH RECURSIVE chain(id, prev, depth) AS ( \
               SELECT id, previous_run_id, 1 FROM automation_runs WHERE id = ? \
               UNION ALL \
               SELECT r.id, r.previous_run_id, c.depth + 1 \
                 FROM automation_runs r JOIN chain c ON c.prev = r.id \
             ) \
             SELECT MAX(depth) AS d FROM chain",
        )
        .bind(run_id.to_string())
        .fetch_one(&self.db)
        .await?;
        Ok(row.get::<i64, _>("d"))
    }

    /// Create a retry run chained to a failed one, emitting `retry_scheduled` on
    /// the previous run and `queued` on the new one. `None` if disabled (also
    /// emits `retry_skipped`).
    pub async fn schedule_retry(
        &self,
        previous_run: &AutomationRun,
        scheduled_for: DateTime<Utc>,
        next_attempt: i64,
    ) -> StateResult<Option<AutomationRun>> {
        // Enabled gate applies only to automation-linked runs; ad-hoc one-time
        // runs (no automation) always retry.
        if let Some(aid) = previous_run.automation_id {
            let enabled = matches!(self.get_automation(aid).await?, Some(a) if a.enabled);
            if !enabled {
                let payload = serde_json::json!({
                    "reason": "automation_disabled",
                    "attempt": next_attempt,
                });
                let now = Self::now();
                let mut tx = self.db.begin().await?;
                Self::insert_log(
                    &mut tx,
                    previous_run.id,
                    RunLogKind::RetrySkipped,
                    Some(&payload),
                    &now,
                )
                .await?;
                tx.commit().await?;
                return Ok(None);
            }
        }

        let run = self
            .insert_run_with_session(NewRun::retry_of(previous_run), scheduled_for, None)
            .await?;

        if let Some(ref retry) = run {
            let payload = serde_json::json!({
                "next_run_id": retry.id.to_string(),
                "scheduled_for": scheduled_for.to_rfc3339(),
                "attempt": next_attempt,
            });
            let now = Self::now();
            let mut tx = self.db.begin().await?;
            Self::insert_log(
                &mut tx,
                previous_run.id,
                RunLogKind::RetryScheduled,
                Some(&payload),
                &now,
            )
            .await?;
            tx.commit().await?;
        }
        Ok(run)
    }

    /// Requeue expired-lease `running` rows, emitting `lease_lost` per row. Boot
    /// recovery passes `None` to requeue *all* running rows unconditionally.
    pub async fn reap_expired_leases(
        &self,
        now: Option<DateTime<Utc>>,
    ) -> StateResult<Vec<Uuid>> {
        let now_s = now.map(|t| t.to_rfc3339());
        let rows = match &now_s {
            Some(ts) => {
                sqlx::query(
                    "SELECT id FROM automation_runs \
                     WHERE status = 'running' AND lease_until IS NOT NULL AND lease_until < ?",
                )
                .bind(ts)
                .fetch_all(&self.db)
                .await?
            }
            None => {
                sqlx::query("SELECT id FROM automation_runs WHERE status = 'running'")
                    .fetch_all(&self.db)
                    .await?
            }
        };
        if rows.is_empty() {
            return Ok(vec![]);
        }
        let updated_at = Self::now();
        let mut reaped = Vec::with_capacity(rows.len());
        for r in &rows {
            let id_s: String = r.get("id");
            let mut tx = self.db.begin().await?;
            let res = sqlx::query(
                "UPDATE automation_runs SET status = 'queued', lease_until = NULL, updated_at = ? \
                 WHERE id = ? AND status = 'running'",
            )
            .bind(&updated_at)
            .bind(&id_s)
            .execute(&mut *tx)
            .await?;
            if res.rows_affected() == 1 {
                let run_id = parse_uuid(id_s.clone(), "automation_runs.id")?;
                Self::insert_log(&mut tx, run_id, RunLogKind::LeaseLost, None, &updated_at).await?;
                tx.commit().await?;
                reaped.push(run_id);
            } else {
                tx.rollback().await?;
            }
        }
        Ok(reaped)
    }

    // ── events ──────────────────────────────────────────────────────────

    pub async fn append_log(
        &self,
        run_id: Uuid,
        kind: RunLogKind,
        payload: Option<&serde_json::Value>,
    ) -> StateResult<()> {
        let now = Self::now();
        let mut tx = self.db.begin().await?;
        Self::insert_log(&mut tx, run_id, kind, payload, &now).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_logs_for_run(&self, run_id: Uuid) -> StateResult<Vec<RunLog>> {
        let rows = sqlx::query(
            "SELECT id, run_id, ts, kind, payload FROM automation_run_logs \
             WHERE run_id = ? ORDER BY ts ASC, id ASC",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.db)
        .await?;
        rows.iter().map(RunLog::from_row).collect()
    }

    /// Shared INSERT used inside the lifecycle transactions.
    async fn insert_log(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        run_id: Uuid,
        kind: RunLogKind,
        payload: Option<&serde_json::Value>,
        ts: &str,
    ) -> StateResult<()> {
        let payload_str = payload.map(serde_json::to_string).transpose()?;
        sqlx::query(
            "INSERT INTO automation_run_logs (run_id, ts, kind, payload) VALUES (?, ?, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(ts)
        .bind(kind.as_str())
        .bind(&payload_str)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn fresh_db() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// Seed a user + its default workspace; returns `(user_id, workspace_id)`.
    async fn seed(pool: &SqlitePool) -> (Uuid, Uuid) {
        let uid = Uuid::new_v4();
        let wid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users \
                 (id, username, password_hash, role, is_active, preferred_language, created_at, updated_at) \
             VALUES (?, ?, 'x', 'user', 1, 'en', ?, ?)",
        )
        .bind(uid.to_string())
        .bind(format!("u-{uid}"))
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, user_id, title, created_at, updated_at) VALUES (?, ?, 'W', ?, ?)",
        )
        .bind(wid.to_string())
        .bind(uid.to_string())
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        (uid, wid)
    }

    #[tokio::test]
    async fn automation_crud_and_trigger() {
        let pool = fresh_db().await;
        let (uid, wid) = seed(&pool).await;
        let state = AutomationsState::new(pool);

        let a = state
            .create_automation(
                wid,
                "daily digest".into(),
                Some("desc".into()),
                "summarize today".into(),
                "coworker".into(),
                None,
                uid,
            )
            .await
            .unwrap();
        assert_eq!(a.prompt, "summarize today");
        assert!(a.enabled);
        assert_eq!(state.list_automations_by_workspace(wid).await.unwrap().len(), 1);

        // Cron trigger with a computed next_fire_at.
        let spec = TriggerSpec::Cron { expr: "0 9 * * *".into(), tz: None };
        let next = crate::cron::next_fire_after("0 9 * * *", "UTC", Utc::now()).unwrap();
        let t = state
            .create_trigger(a.id, &spec, true, Some(next), None)
            .await
            .unwrap();
        assert_eq!(t.kind, TriggerKind::Cron);
        assert_eq!(state.list_triggers(a.id).await.unwrap().len(), 1);

        // Update: disable, which cancels queued runs.
        let updated = state
            .update_automation(a.id, None, None, None, None, None, Some(false))
            .await
            .unwrap();
        assert!(!updated.enabled);
    }

    #[tokio::test]
    async fn webhook_trigger_lookup_and_run() {
        let pool = fresh_db().await;
        let (uid, wid) = seed(&pool).await;
        let state = AutomationsState::new(pool);

        let a = state
            .create_automation(wid, "hooked".into(), None, "go".into(), "coworker".into(), None, uid)
            .await
            .unwrap();
        let (token, hash) = AutomationsState::new_webhook_token();
        let t = state
            .create_trigger(a.id, &TriggerSpec::Webhook {}, true, None, Some(hash))
            .await
            .unwrap();
        assert_eq!(t.kind, TriggerKind::Webhook);

        // Lookup by token hash: correct → Some(trigger); wrong → None.
        assert_eq!(
            state
                .find_trigger_by_webhook_token_hash(&AutomationsState::webhook_token_hash(&token))
                .await
                .unwrap()
                .map(|f| f.id),
            Some(t.id)
        );
        assert!(
            state
                .find_trigger_by_webhook_token_hash(&AutomationsState::webhook_token_hash("wrong"))
                .await
                .unwrap()
                .is_none()
        );

        // Fire it: a run is created for the automation.
        let run = state.create_webhook_run(&a, t.id).await.unwrap().unwrap();
        assert_eq!(run.trigger_id, Some(t.id));
        assert_eq!(run.automation_id, Some(a.id));

        // Disabled automation gates run creation.
        state
            .update_automation(a.id, None, None, None, None, None, Some(false))
            .await
            .unwrap();
        assert!(state.create_webhook_run(&a, t.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn run_lifecycle_claim_finalize() {
        let pool = fresh_db().await;
        let (uid, wid) = seed(&pool).await;
        let state = AutomationsState::new(pool);

        let a = state
            .create_automation(wid, "job".into(), None, "do it".into(), "coworker".into(), None, uid)
            .await
            .unwrap();

        // Manual run → queued, with triggered + queued events.
        let run = state.create_scheduled_run(&a, Utc::now(), "manual").await.unwrap();
        assert_eq!(run.status, RunStatus::Queued);
        let events = state.list_logs_for_run(run.id).await.unwrap();
        let kinds: Vec<_> = events.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![RunLogKind::Triggered, RunLogKind::Queued]);

        // Claim → running.
        let now = Utc::now() + chrono::Duration::seconds(1);
        let lease = now + chrono::Duration::minutes(3);
        let claimed = state.claim_due_run(now, lease).await.unwrap().expect("claimable");
        assert_eq!(claimed.id, run.id);
        assert_eq!(claimed.status, RunStatus::Running);
        // Nothing left to claim.
        assert!(state.claim_due_run(now, lease).await.unwrap().is_none());

        // Finalize succeeded.
        assert!(state
            .finalize_run(run.id, RunStatus::Succeeded, RunLogKind::Succeeded, None)
            .await
            .unwrap());
        assert_eq!(state.get_run(run.id).await.unwrap().unwrap().status, RunStatus::Succeeded);
        assert!(state
            .list_logs_for_run(run.id)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == RunLogKind::Succeeded));

        // A second finalize finds no running row.
        assert!(!state
            .finalize_run(run.id, RunStatus::Failed, RunLogKind::Failed, None)
            .await
            .unwrap());

        // Deleting the automation preserves the run as an audit record: its
        // automation_id is SET NULL, and its snapshot (prompt) stays intact.
        assert!(state.delete_automation(a.id).await.unwrap());
        let preserved = state.get_run(run.id).await.unwrap().expect("run preserved");
        assert!(preserved.automation_id.is_none(), "automation_id should be NULL");
        assert_eq!(preserved.prompt, "do it");
    }

    #[tokio::test]
    async fn adhoc_one_time_run() {
        let pool = fresh_db().await;
        let (uid, wid) = seed(&pool).await;
        let state = AutomationsState::new(pool);

        // Schedule one hour out — self-contained, no automation.
        let future = Utc::now() + chrono::Duration::hours(1);
        let run = state
            .create_adhoc_run(wid, "t".into(), "hello".into(), "coworker".into(), None, future, uid)
            .await
            .unwrap();
        assert!(run.automation_id.is_none());
        assert_eq!(run.prompt, "hello");
        assert_eq!(run.status, RunStatus::Queued);

        // Not due yet; becomes claimable only once the scheduled instant passes.
        let lease = Utc::now() + chrono::Duration::minutes(3);
        assert!(state.claim_due_run(Utc::now(), lease).await.unwrap().is_none());
        let claimed = state
            .claim_due_run(future + chrono::Duration::seconds(1), lease)
            .await
            .unwrap()
            .expect("claimable once due");
        assert_eq!(claimed.id, run.id);
        assert_eq!(claimed.prompt, "hello");

        // Full audit trail exists without any automation.
        let kinds: Vec<_> = state
            .list_logs_for_run(run.id)
            .await
            .unwrap()
            .iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, vec![RunLogKind::Triggered, RunLogKind::Queued]);
    }

    #[tokio::test]
    async fn cron_fire_advances_and_gates_on_enabled() {
        let pool = fresh_db().await;
        let (uid, wid) = seed(&pool).await;
        let state = AutomationsState::new(pool);
        let a = state
            .create_automation(wid, "job".into(), None, "do it".into(), "coworker".into(), None, uid)
            .await
            .unwrap();
        let due = Utc::now();
        let next = due + chrono::Duration::minutes(5);
        let spec = TriggerSpec::Cron { expr: "*/5 * * * *".into(), tz: None };
        let t = state.create_trigger(a.id, &spec, true, Some(due), None).await.unwrap();
        let payload = serde_json::json!({ "source": "cron" });

        // Enabled: run created and next_fire_at advanced, atomically in one tx.
        assert!(state.fire_cron_trigger(&a, t.id, due, next, &payload).await.unwrap().is_some());
        assert_eq!(state.list_runs(a.id).await.unwrap().len(), 1);
        let advanced = state.get_trigger(t.id).await.unwrap().unwrap().next_fire_at;
        assert!(advanced.unwrap() > due, "next_fire_at should have advanced");

        // Disabled (rare fire-vs-disable race): no run, and the advance rolls back.
        let next2 = next + chrono::Duration::minutes(5);
        state
            .update_automation(a.id, None, None, None, None, None, Some(false))
            .await
            .unwrap();
        assert!(state.fire_cron_trigger(&a, t.id, next, next2, &payload).await.unwrap().is_none());
        assert_eq!(state.list_runs(a.id).await.unwrap().len(), 1);
        assert_eq!(
            state.get_trigger(t.id).await.unwrap().unwrap().next_fire_at,
            advanced,
            "disabled fire must roll back the advance"
        );
    }

    #[tokio::test]
    async fn automation_run_session_tagged_automation_origin() {
        let pool = fresh_db().await;
        let (uid, wid) = seed(&pool).await;
        let state = AutomationsState::new(pool.clone());
        let a = state
            .create_automation(wid, "j".into(), None, "p".into(), "coworker".into(), None, uid)
            .await
            .unwrap();
        let run = state.create_scheduled_run(&a, Utc::now(), "manual").await.unwrap();
        let origin: String = sqlx::query_scalar("SELECT origin FROM sessions WHERE id = ?")
            .bind(run.session_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(origin, "automation");
    }

    #[tokio::test]
    async fn deleting_run_session_cascades_run_and_logs() {
        let pool = fresh_db().await;
        let (uid, wid) = seed(&pool).await;
        let state = AutomationsState::new(pool.clone());
        let a = state
            .create_automation(wid, "j".into(), None, "p".into(), "coworker".into(), None, uid)
            .await
            .unwrap();
        let run = state.create_scheduled_run(&a, Utc::now(), "manual").await.unwrap();

        // Deleting the run's session (how delete_run tears a run down) cascades
        // the run row and its logs atomically.
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(run.session_id.to_string())
            .execute(&pool)
            .await
            .unwrap();
        assert!(state.get_run(run.id).await.unwrap().is_none());
        let logs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM automation_run_logs WHERE run_id = ?")
            .bind(run.id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(logs, 0);
    }

    #[tokio::test]
    async fn reap_requeues_expired_lease() {
        let pool = fresh_db().await;
        let (uid, wid) = seed(&pool).await;
        let state = AutomationsState::new(pool);
        let a = state
            .create_automation(wid, "j".into(), None, "p".into(), "coworker".into(), None, uid)
            .await
            .unwrap();
        let run = state.create_scheduled_run(&a, Utc::now(), "manual").await.unwrap();

        // Claim with an already-expired lease, then reap.
        let past = Utc::now() - chrono::Duration::minutes(10);
        state.claim_due_run(Utc::now(), past).await.unwrap().unwrap();
        let reaped = state.reap_expired_leases(Some(Utc::now())).await.unwrap();
        assert_eq!(reaped, vec![run.id]);
        assert_eq!(state.get_run(run.id).await.unwrap().unwrap().status, RunStatus::Queued);
        assert!(state
            .list_logs_for_run(run.id)
            .await
            .unwrap()
            .iter()
            .any(|e| e.kind == RunLogKind::LeaseLost));
    }
}
