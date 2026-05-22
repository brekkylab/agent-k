use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::SqliteRepository;
use crate::{
    model::{EventKind, RunStatus, TriggerKind, TriggerSpec},
    repository::{RepositoryError, RepositoryResult},
};

// Db structs

#[derive(Debug, Clone)]
pub struct DbAutomation {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub prompts: Vec<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DbAutomationTrigger {
    pub id: Uuid,
    pub automation_id: Uuid,
    pub kind: TriggerKind,
    pub spec_json: String,
    pub enabled: bool,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub webhook_token_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DbAutomationRun {
    pub id: Uuid,
    pub automation_id: Uuid,
    pub trigger_id: Option<Uuid>,
    pub session_id: Uuid,
    pub status: RunStatus,
    pub scheduled_for: DateTime<Utc>,
    pub lease_until: Option<DateTime<Utc>>,
    pub previous_run_id: Option<Uuid>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DbAutomationRunEvent {
    pub id: i64,
    pub run_id: Uuid,
    pub ts: DateTime<Utc>,
    pub kind: EventKind,
    pub payload: Option<serde_json::Value>,
}

// Row → Db helpers

impl SqliteRepository {
    fn row_to_db_automation(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbAutomation> {
        let prompts_json: String = row.get("prompts_json");
        let prompts: Vec<String> = serde_json::from_str(&prompts_json)?;
        Ok(DbAutomation {
            id: Self::parse_uuid(row.get::<String, _>("id"), "automations.id")?,
            project_id: Self::parse_uuid(
                row.get::<String, _>("project_id"),
                "automations.project_id",
            )?,
            name: row.get("name"),
            description: row.get("description"),
            prompts,
            created_by: Self::parse_uuid(
                row.get::<String, _>("created_by"),
                "automations.created_by",
            )?,
            created_at: Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "automations.created_at",
            )?,
            updated_at: Self::parse_timestamp(
                row.get::<String, _>("updated_at"),
                "automations.updated_at",
            )?,
        })
    }

    fn row_to_db_trigger(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbAutomationTrigger> {
        let kind_str: String = row.get("kind");
        let kind = TriggerKind::from_str(&kind_str).ok_or_else(|| {
            RepositoryError::InvalidData(format!("invalid trigger kind '{kind_str}'"))
        })?;
        let next_fire_at = row
            .get::<Option<String>, _>("next_fire_at")
            .map(|s| Self::parse_timestamp(s, "automation_triggers.next_fire_at"))
            .transpose()?;
        Ok(DbAutomationTrigger {
            id: Self::parse_uuid(row.get::<String, _>("id"), "automation_triggers.id")?,
            automation_id: Self::parse_uuid(
                row.get::<String, _>("automation_id"),
                "automation_triggers.automation_id",
            )?,
            kind,
            spec_json: row.get("spec_json"),
            enabled: row.get::<i64, _>("enabled") != 0,
            next_fire_at,
            webhook_token_hash: row.get("webhook_token_hash"),
            created_at: Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "automation_triggers.created_at",
            )?,
            updated_at: Self::parse_timestamp(
                row.get::<String, _>("updated_at"),
                "automation_triggers.updated_at",
            )?,
        })
    }

    fn row_to_db_run(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbAutomationRun> {
        let status_str: String = row.get("status");
        let status = RunStatus::from_str(&status_str).ok_or_else(|| {
            RepositoryError::InvalidData(format!("invalid run status '{status_str}'"))
        })?;
        let trigger_id = row
            .get::<Option<String>, _>("trigger_id")
            .map(|s| Self::parse_uuid(s, "automation_runs.trigger_id"))
            .transpose()?;
        let previous_run_id = row
            .get::<Option<String>, _>("previous_run_id")
            .map(|s| Self::parse_uuid(s, "automation_runs.previous_run_id"))
            .transpose()?;
        let lease_until = row
            .get::<Option<String>, _>("lease_until")
            .map(|s| Self::parse_timestamp(s, "automation_runs.lease_until"))
            .transpose()?;
        Ok(DbAutomationRun {
            id: Self::parse_uuid(row.get::<String, _>("id"), "automation_runs.id")?,
            automation_id: Self::parse_uuid(
                row.get::<String, _>("automation_id"),
                "automation_runs.automation_id",
            )?,
            trigger_id,
            session_id: Self::parse_uuid(
                row.get::<String, _>("session_id"),
                "automation_runs.session_id",
            )?,
            status,
            scheduled_for: Self::parse_timestamp(
                row.get::<String, _>("scheduled_for"),
                "automation_runs.scheduled_for",
            )?,
            lease_until,
            previous_run_id,
            idempotency_key: row.get("idempotency_key"),
            created_at: Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "automation_runs.created_at",
            )?,
            updated_at: Self::parse_timestamp(
                row.get::<String, _>("updated_at"),
                "automation_runs.updated_at",
            )?,
        })
    }

    fn row_to_db_event(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbAutomationRunEvent> {
        let kind_str: String = row.get("kind");
        let kind = EventKind::from_str(&kind_str).ok_or_else(|| {
            RepositoryError::InvalidData(format!("invalid event kind '{kind_str}'"))
        })?;
        let payload = row
            .get::<Option<String>, _>("payload")
            .map(|s| serde_json::from_str::<serde_json::Value>(&s))
            .transpose()?;
        Ok(DbAutomationRunEvent {
            id: row.get("id"),
            run_id: Self::parse_uuid(
                row.get::<String, _>("run_id"),
                "automation_run_events.run_id",
            )?,
            ts: Self::parse_timestamp(
                row.get::<String, _>("ts"),
                "automation_run_events.ts",
            )?,
            kind,
            payload,
        })
    }

    fn ts_string(ts: DateTime<Utc>) -> String {
        ts.to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    // automations

    pub async fn create_automation(
        &self,
        project_id: Uuid,
        name: String,
        description: Option<String>,
        prompts: Vec<String>,
        created_by: Uuid,
    ) -> RepositoryResult<DbAutomation> {
        let id = Uuid::new_v4();
        let now = Self::now_string();
        let prompts_json = serde_json::to_string(&prompts)?;
        sqlx::query(
            "INSERT INTO automations (id, project_id, name, description, prompts_json, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(project_id.to_string())
        .bind(&name)
        .bind(&description)
        .bind(&prompts_json)
        .bind(created_by.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(DbAutomation {
            id,
            project_id,
            name,
            description,
            prompts,
            created_by,
            created_at: Self::parse_timestamp(now.clone(), "automations.created_at")?,
            updated_at: Self::parse_timestamp(now, "automations.updated_at")?,
        })
    }

    pub async fn get_automation(&self, id: Uuid) -> RepositoryResult<Option<DbAutomation>> {
        let row = sqlx::query(
            "SELECT id, project_id, name, description, prompts_json, created_by, created_at, updated_at \
             FROM automations WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_db_automation).transpose()
    }

    pub async fn list_automations_in_project(
        &self,
        project_id: Uuid,
    ) -> RepositoryResult<Vec<DbAutomation>> {
        let rows = sqlx::query(
            "SELECT id, project_id, name, description, prompts_json, created_by, created_at, updated_at \
             FROM automations WHERE project_id = ? ORDER BY created_at DESC",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_db_automation).collect()
    }

    /// All automations across projects the user owns or is a member of.
    pub async fn list_automations_for_user(
        &self,
        requesting_user_id: Uuid,
    ) -> RepositoryResult<Vec<DbAutomation>> {
        let uid = requesting_user_id.to_string();
        let rows = sqlx::query(
            "SELECT a.id, a.project_id, a.name, a.description, a.prompts_json, a.created_by, a.created_at, a.updated_at
             FROM automations a
             JOIN projects p ON p.id = a.project_id
             WHERE p.owner_id = ?1
                OR EXISTS (SELECT 1 FROM project_members pm
                           WHERE pm.project_id = a.project_id AND pm.user_id = ?1)
             ORDER BY a.created_at DESC",
        )
        .bind(&uid)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_db_automation).collect()
    }

    pub async fn update_automation(
        &self,
        id: Uuid,
        name: Option<String>,
        description: Option<Option<String>>,
        prompts: Option<Vec<String>>,
    ) -> RepositoryResult<DbAutomation> {
        let current = self.get_automation(id).await?.ok_or_else(|| {
            RepositoryError::InvalidData(format!("automation {id} not found"))
        })?;
        let new_name = name.unwrap_or(current.name);
        let new_desc = description.unwrap_or(current.description);
        let new_prompts = prompts.unwrap_or(current.prompts);
        let new_prompts_json = serde_json::to_string(&new_prompts)?;
        let now = Self::now_string();

        sqlx::query(
            "UPDATE automations SET name = ?, description = ?, prompts_json = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&new_name)
        .bind(&new_desc)
        .bind(&new_prompts_json)
        .bind(&now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        self.get_automation(id).await?.ok_or_else(|| {
            RepositoryError::InvalidData("automation disappeared after update".into())
        })
    }

    pub async fn delete_automation(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM automations WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // automation_triggers

    pub async fn create_trigger(
        &self,
        automation_id: Uuid,
        spec: &TriggerSpec,
        webhook_token_hash: Option<String>,
        next_fire_at: Option<DateTime<Utc>>,
    ) -> RepositoryResult<DbAutomationTrigger> {
        let id = Uuid::new_v4();
        let now = Self::now_string();
        let kind = spec.kind();
        let spec_json = spec.to_db_spec_json()?;
        let nfa_str = next_fire_at.map(Self::ts_string);

        sqlx::query(
            "INSERT INTO automation_triggers \
               (id, automation_id, kind, spec_json, enabled, next_fire_at, webhook_token_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 1, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(automation_id.to_string())
        .bind(kind.as_str())
        .bind(&spec_json)
        .bind(&nfa_str)
        .bind(&webhook_token_hash)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| Self::map_db_error(e, "automation_triggers.webhook_token_hash"))?;

        Ok(DbAutomationTrigger {
            id,
            automation_id,
            kind,
            spec_json,
            enabled: true,
            next_fire_at,
            webhook_token_hash,
            created_at: Self::parse_timestamp(now.clone(), "automation_triggers.created_at")?,
            updated_at: Self::parse_timestamp(now, "automation_triggers.updated_at")?,
        })
    }

    pub async fn get_trigger(&self, id: Uuid) -> RepositoryResult<Option<DbAutomationTrigger>> {
        let row = sqlx::query(
            "SELECT id, automation_id, kind, spec_json, enabled, next_fire_at, webhook_token_hash, created_at, updated_at \
             FROM automation_triggers WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_db_trigger).transpose()
    }

    pub async fn list_triggers_for_automation(
        &self,
        automation_id: Uuid,
    ) -> RepositoryResult<Vec<DbAutomationTrigger>> {
        let rows = sqlx::query(
            "SELECT id, automation_id, kind, spec_json, enabled, next_fire_at, webhook_token_hash, created_at, updated_at \
             FROM automation_triggers WHERE automation_id = ? ORDER BY created_at ASC",
        )
        .bind(automation_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_db_trigger).collect()
    }

    pub async fn update_trigger(
        &self,
        id: Uuid,
        spec: Option<&TriggerSpec>,
        enabled: Option<bool>,
        next_fire_at: Option<Option<DateTime<Utc>>>,
    ) -> RepositoryResult<DbAutomationTrigger> {
        let current = self
            .get_trigger(id)
            .await?
            .ok_or_else(|| RepositoryError::InvalidData(format!("trigger {id} not found")))?;

        let (new_kind, new_spec_json) = match spec {
            Some(s) => (s.kind(), s.to_db_spec_json()?),
            None => (current.kind, current.spec_json.clone()),
        };
        let new_enabled = enabled.unwrap_or(current.enabled);
        let new_next = next_fire_at.unwrap_or(current.next_fire_at);
        let nfa_str = new_next.map(Self::ts_string);
        let now = Self::now_string();

        sqlx::query(
            "UPDATE automation_triggers \
               SET kind = ?, spec_json = ?, enabled = ?, next_fire_at = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(new_kind.as_str())
        .bind(&new_spec_json)
        .bind(if new_enabled { 1i64 } else { 0i64 })
        .bind(&nfa_str)
        .bind(&now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        self.get_trigger(id).await?.ok_or_else(|| {
            RepositoryError::InvalidData("trigger disappeared after update".into())
        })
    }

    pub async fn delete_trigger(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM automation_triggers WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Returns the most recent run for `trigger_id` whose `idempotency_key`
    /// matches. Rows whose key has been NULL'd by the housekeeper cleanup are
    /// implicitly excluded by the `=` filter. Used by the webhook handler to
    /// replay a cached response when the caller resends the same
    /// `Idempotency-Key`.
    pub async fn find_webhook_run_by_idempotency_key(
        &self,
        trigger_id: Uuid,
        idempotency_key: &str,
    ) -> RepositoryResult<Option<DbAutomationRun>> {
        let row = sqlx::query(
            "SELECT id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at \
             FROM automation_runs \
             WHERE trigger_id = ? \
               AND idempotency_key = ? \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(trigger_id.to_string())
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_db_run).transpose()
    }

    /// NULLs `idempotency_key` for runs created before `cutoff`. This releases
    /// the UNIQUE partial index slot so the caller may safely reuse the same
    /// `Idempotency-Key` after the retention window expires. Returns the
    /// number of rows affected.
    pub async fn clear_expired_idempotency_keys(
        &self,
        cutoff: DateTime<Utc>,
    ) -> RepositoryResult<u64> {
        let cutoff_s = Self::ts_string(cutoff);
        let now = Self::now_string();
        let result = sqlx::query(
            "UPDATE automation_runs SET idempotency_key = NULL, updated_at = ? \
             WHERE idempotency_key IS NOT NULL AND created_at < ?",
        )
        .bind(&now)
        .bind(&cutoff_s)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn find_trigger_by_webhook_token_hash(
        &self,
        token_hash: &str,
    ) -> RepositoryResult<Option<DbAutomationTrigger>> {
        let row = sqlx::query(
            "SELECT id, automation_id, kind, spec_json, enabled, next_fire_at, webhook_token_hash, created_at, updated_at \
             FROM automation_triggers WHERE webhook_token_hash = ?",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_db_trigger).transpose()
    }

    pub async fn list_due_cron_triggers(
        &self,
        now: DateTime<Utc>,
    ) -> RepositoryResult<Vec<DbAutomationTrigger>> {
        let now_s = Self::ts_string(now);
        let rows = sqlx::query(
            "SELECT id, automation_id, kind, spec_json, enabled, next_fire_at, webhook_token_hash, created_at, updated_at \
             FROM automation_triggers \
             WHERE enabled = 1 AND kind = 'cron' AND next_fire_at IS NOT NULL AND next_fire_at <= ? \
             ORDER BY next_fire_at ASC",
        )
        .bind(&now_s)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_db_trigger).collect()
    }

    // automation_runs

    pub async fn create_run(
        &self,
        automation_id: Uuid,
        trigger_id: Option<Uuid>,
        session_id: Uuid,
        scheduled_for: DateTime<Utc>,
        previous_run_id: Option<Uuid>,
    ) -> RepositoryResult<DbAutomationRun> {
        let id = Uuid::new_v4();
        let now = Self::now_string();
        let scheduled_s = Self::ts_string(scheduled_for);

        sqlx::query(
            "INSERT INTO automation_runs \
               (id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'queued', ?, NULL, ?, NULL, ?, ?)",
        )
        .bind(id.to_string())
        .bind(automation_id.to_string())
        .bind(trigger_id.map(|u| u.to_string()))
        .bind(session_id.to_string())
        .bind(&scheduled_s)
        .bind(previous_run_id.map(|u| u.to_string()))
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| Self::map_db_error(e, "automation_runs.session_id"))?;

        Ok(DbAutomationRun {
            id,
            automation_id,
            trigger_id,
            session_id,
            status: RunStatus::Queued,
            scheduled_for,
            lease_until: None,
            previous_run_id,
            idempotency_key: None,
            created_at: Self::parse_timestamp(now.clone(), "automation_runs.created_at")?,
            updated_at: Self::parse_timestamp(now, "automation_runs.updated_at")?,
        })
    }

    /// Atomically create a fresh automation session, the queued run referencing
    /// it, and the canonical `triggered` + `queued` events — all in one tx so
    /// partial failure can't leave an orphan session or a run without its
    /// initial event log. Used by manual run creation and webhook firing.
    pub async fn create_automation_run_with_session(
        &self,
        automation_id: Uuid,
        project_id: Uuid,
        creator_id: Uuid,
        trigger_id: Option<Uuid>,
        scheduled_for: DateTime<Utc>,
        previous_run_id: Option<Uuid>,
        triggered_payload: Option<&serde_json::Value>,
        idempotency_key: Option<&str>,
    ) -> RepositoryResult<DbAutomationRun> {
        let mut tx = self.pool.begin().await?;
        let now = Self::now_string();
        let scheduled_s = Self::ts_string(scheduled_for);

        let session_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO sessions (id, project_id, creator_id, share_mode, origin, created_at, updated_at) \
             VALUES (?, ?, ?, 'private', 'automation', ?, ?)",
        )
        .bind(session_id.to_string())
        .bind(project_id.to_string())
        .bind(creator_id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO automation_runs \
               (id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'queued', ?, NULL, ?, ?, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(automation_id.to_string())
        .bind(trigger_id.map(|u| u.to_string()))
        .bind(session_id.to_string())
        .bind(&scheduled_s)
        .bind(previous_run_id.map(|u| u.to_string()))
        .bind(idempotency_key)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| Self::map_db_error(e, "automation_runs.idempotency_key"))?;

        let triggered_str = triggered_payload.map(serde_json::to_string).transpose()?;
        sqlx::query(
            "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
             VALUES (?, ?, 'triggered', ?)",
        )
        .bind(run_id.to_string())
        .bind(&now)
        .bind(&triggered_str)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
             VALUES (?, ?, 'queued', NULL)",
        )
        .bind(run_id.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(DbAutomationRun {
            id: run_id,
            automation_id,
            trigger_id,
            session_id,
            status: RunStatus::Queued,
            scheduled_for,
            lease_until: None,
            previous_run_id,
            idempotency_key: idempotency_key.map(|s| s.to_string()),
            created_at: Self::parse_timestamp(now.clone(), "automation_runs.created_at")?,
            updated_at: Self::parse_timestamp(now, "automation_runs.updated_at")?,
        })
    }

    /// Cron-trigger firing: session + run + initial events + advance the
    /// trigger's `next_fire_at` to the post-fire value, all in one tx. This
    /// guarantees the trigger never stays stuck on a past `next_fire_at` while
    /// the run was successfully queued (which would re-fire on the next tick).
    pub async fn fire_cron_trigger(
        &self,
        automation_id: Uuid,
        project_id: Uuid,
        creator_id: Uuid,
        trigger_id: Uuid,
        scheduled_for: DateTime<Utc>,
        next_fire_at: DateTime<Utc>,
        triggered_payload: &serde_json::Value,
    ) -> RepositoryResult<DbAutomationRun> {
        let mut tx = self.pool.begin().await?;
        let now = Self::now_string();
        let scheduled_s = Self::ts_string(scheduled_for);
        let next_s = Self::ts_string(next_fire_at);

        let session_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO sessions (id, project_id, creator_id, share_mode, origin, created_at, updated_at) \
             VALUES (?, ?, ?, 'private', 'automation', ?, ?)",
        )
        .bind(session_id.to_string())
        .bind(project_id.to_string())
        .bind(creator_id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO automation_runs \
               (id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'queued', ?, NULL, NULL, NULL, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(automation_id.to_string())
        .bind(trigger_id.to_string())
        .bind(session_id.to_string())
        .bind(&scheduled_s)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| Self::map_db_error(e, "automation_runs.session_id"))?;

        let triggered_str = serde_json::to_string(triggered_payload)?;
        sqlx::query(
            "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
             VALUES (?, ?, 'triggered', ?)",
        )
        .bind(run_id.to_string())
        .bind(&now)
        .bind(&triggered_str)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
             VALUES (?, ?, 'queued', NULL)",
        )
        .bind(run_id.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE automation_triggers SET next_fire_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&next_s)
        .bind(&now)
        .bind(trigger_id.to_string())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(DbAutomationRun {
            id: run_id,
            automation_id,
            trigger_id: Some(trigger_id),
            session_id,
            status: RunStatus::Queued,
            scheduled_for,
            lease_until: None,
            previous_run_id: None,
            idempotency_key: None,
            created_at: Self::parse_timestamp(now.clone(), "automation_runs.created_at")?,
            updated_at: Self::parse_timestamp(now, "automation_runs.updated_at")?,
        })
    }

    pub async fn get_run(&self, id: Uuid) -> RepositoryResult<Option<DbAutomationRun>> {
        let row = sqlx::query(
            "SELECT id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at \
             FROM automation_runs WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_db_run).transpose()
    }

    pub async fn list_runs_for_automation(
        &self,
        automation_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> RepositoryResult<Vec<DbAutomationRun>> {
        let rows = sqlx::query(
            "SELECT id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at \
             FROM automation_runs WHERE automation_id = ? \
             ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(automation_id.to_string())
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_db_run).collect()
    }

    /// Atomic queued→running pickup. Single UPDATE+RETURNING relies on
    /// SQLite's writer lock to serialise concurrent claims.
    pub async fn claim_due_run(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> RepositoryResult<Option<DbAutomationRun>> {
        let now_s = Self::ts_string(now);
        let lease_s = Self::ts_string(lease_until);
        let updated_at = Self::now_string();

        let row = sqlx::query(
            "UPDATE automation_runs \
                SET status = 'running', lease_until = ?1, updated_at = ?2 \
              WHERE id = ( \
                SELECT id FROM automation_runs \
                 WHERE status = 'queued' AND scheduled_for <= ?3 \
                 ORDER BY scheduled_for ASC LIMIT 1 \
              ) \
             RETURNING id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at",
        )
        .bind(&lease_s)
        .bind(&updated_at)
        .bind(&now_s)
        .fetch_optional(&self.pool)
        .await?;

        row.as_ref().map(Self::row_to_db_run).transpose()
    }

    /// Push the lease forward. Returns false if we no longer own the run
    /// (reaped or terminated) — caller should cancel work.
    pub async fn renew_lease(
        &self,
        run_id: Uuid,
        new_lease_until: DateTime<Utc>,
    ) -> RepositoryResult<bool> {
        let now = Self::now_string();
        let lease_s = Self::ts_string(new_lease_until);
        let res = sqlx::query(
            "UPDATE automation_runs SET lease_until = ?, updated_at = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(&lease_s)
        .bind(&now)
        .bind(run_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Atomically write the terminal event and update the run status (clearing
    /// the lease). The UPDATE is guarded by `status='running'` — if the row
    /// has been reaped out from under us, no rows match, the tx is rolled
    /// back, and `Ok(false)` is returned. Caller can ignore the false case
    /// because the reaper has already written its own lease_lost + queued.
    pub async fn finalize_run(
        &self,
        run_id: Uuid,
        status: RunStatus,
        event_kind: EventKind,
        event_payload: Option<&serde_json::Value>,
    ) -> RepositoryResult<bool> {
        let mut tx = self.pool.begin().await?;
        let now = Self::now_string();

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

        let payload_str = event_payload.map(serde_json::to_string).transpose()?;
        sqlx::query(
            "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(&now)
        .bind(event_kind.as_str())
        .bind(&payload_str)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Returns the attempt number of `run_id`, derived from the
    /// `previous_run_id` chain depth. The initial run in a chain has attempt
    /// 1; each retry increments by one. This is a derived property, not a
    /// persisted column — `DbAutomationRun` stays a 1:1 row mirror.
    pub async fn compute_run_attempt(&self, run_id: Uuid) -> RepositoryResult<i64> {
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
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("d"))
    }

    /// Atomically schedule a retry for a finalized-failed run: creates a fresh
    /// session + queued run with `previous_run_id` pointing back and
    /// `attempt = next_attempt`, and emits `retry_scheduled` on the previous
    /// run. The new run inherits `automation_id` / `trigger_id` (so cron and
    /// webhook context survive the chain). `project_id` and `creator_id` are
    /// looked up from the automation in the same transaction.
    ///
    /// No external trigger payload is emitted on the retry run — only `queued`
    /// — since the chain via `previous_run_id` already documents the origin.
    pub async fn schedule_retry(
        &self,
        previous_run: &DbAutomationRun,
        scheduled_for: DateTime<Utc>,
        next_attempt: i64,
    ) -> RepositoryResult<DbAutomationRun> {
        let mut tx = self.pool.begin().await?;
        let now = Self::now_string();
        let scheduled_s = Self::ts_string(scheduled_for);

        let row = sqlx::query("SELECT project_id, created_by FROM automations WHERE id = ?")
            .bind(previous_run.automation_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        let project_id = Self::parse_uuid(row.get::<String, _>("project_id"), "automations.project_id")?;
        let creator_id = Self::parse_uuid(row.get::<String, _>("created_by"), "automations.created_by")?;

        let session_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO sessions (id, project_id, creator_id, share_mode, origin, created_at, updated_at) \
             VALUES (?, ?, ?, 'private', 'automation', ?, ?)",
        )
        .bind(session_id.to_string())
        .bind(project_id.to_string())
        .bind(creator_id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO automation_runs \
               (id, automation_id, trigger_id, session_id, status, scheduled_for, lease_until, previous_run_id, idempotency_key, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'queued', ?, NULL, ?, NULL, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(previous_run.automation_id.to_string())
        .bind(previous_run.trigger_id.map(|u| u.to_string()))
        .bind(session_id.to_string())
        .bind(&scheduled_s)
        .bind(previous_run.id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| Self::map_db_error(e, "automation_runs.session_id"))?;

        sqlx::query(
            "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
             VALUES (?, ?, 'queued', NULL)",
        )
        .bind(run_id.to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        let retry_payload = serde_json::json!({
            "next_run_id": run_id.to_string(),
            "scheduled_for": scheduled_for,
            "attempt": next_attempt,
        });
        let retry_payload_str = serde_json::to_string(&retry_payload)?;
        sqlx::query(
            "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
             VALUES (?, ?, 'retry_scheduled', ?)",
        )
        .bind(previous_run.id.to_string())
        .bind(&now)
        .bind(&retry_payload_str)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(DbAutomationRun {
            id: run_id,
            automation_id: previous_run.automation_id,
            trigger_id: previous_run.trigger_id,
            session_id,
            status: RunStatus::Queued,
            scheduled_for,
            lease_until: None,
            previous_run_id: Some(previous_run.id),
            idempotency_key: None,
            created_at: Self::parse_timestamp(now.clone(), "automation_runs.created_at")?,
            updated_at: Self::parse_timestamp(now, "automation_runs.updated_at")?,
        })
    }

    /// Move all `running` rows back to `queued` unconditionally and emit
    /// `lease_lost` in the same per-row transaction. Intended for boot-time
    /// recovery — any `running` rows after a process restart are orphaned,
    /// since no in-process worker can still own them.
    pub async fn reap_all_running(&self) -> RepositoryResult<Vec<Uuid>> {
        let rows = sqlx::query("SELECT id FROM automation_runs WHERE status = 'running'")
            .fetch_all(&self.pool)
            .await?;
        if rows.is_empty() {
            return Ok(vec![]);
        }
        let updated_at = Self::now_string();
        let mut reaped = Vec::with_capacity(rows.len());
        for r in &rows {
            let id_s: String = r.get("id");
            let mut tx = self.pool.begin().await?;
            let res = sqlx::query(
                "UPDATE automation_runs SET status = 'queued', lease_until = NULL, updated_at = ? \
                 WHERE id = ? AND status = 'running'",
            )
            .bind(&updated_at)
            .bind(&id_s)
            .execute(&mut *tx)
            .await?;
            if res.rows_affected() == 1 {
                sqlx::query(
                    "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
                     VALUES (?, ?, 'lease_lost', NULL)",
                )
                .bind(&id_s)
                .bind(&updated_at)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                reaped.push(Self::parse_uuid(id_s, "automation_runs.id")?);
            } else {
                tx.rollback().await?;
            }
        }
        Ok(reaped)
    }

    /// Move expired-lease runs back to `queued` and emit `lease_lost` in the
    /// same per-row transaction. Returns reaped run IDs.
    pub async fn reap_expired_leases(&self, now: DateTime<Utc>) -> RepositoryResult<Vec<Uuid>> {
        let now_s = Self::ts_string(now);
        let rows = sqlx::query(
            "SELECT id FROM automation_runs \
             WHERE status = 'running' AND lease_until IS NOT NULL AND lease_until < ?",
        )
        .bind(&now_s)
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(vec![]);
        }

        let mut reaped = Vec::with_capacity(rows.len());
        let updated_at = Self::now_string();
        for r in &rows {
            let id_s: String = r.get("id");
            let mut tx = self.pool.begin().await?;
            let res = sqlx::query(
                "UPDATE automation_runs SET status = 'queued', lease_until = NULL, updated_at = ? \
                 WHERE id = ? AND status = 'running' AND lease_until < ?",
            )
            .bind(&updated_at)
            .bind(&id_s)
            .bind(&now_s)
            .execute(&mut *tx)
            .await?;
            if res.rows_affected() == 1 {
                sqlx::query(
                    "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
                     VALUES (?, ?, 'lease_lost', NULL)",
                )
                .bind(&id_s)
                .bind(&updated_at)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                reaped.push(Self::parse_uuid(id_s, "automation_runs.id")?);
            } else {
                tx.rollback().await?;
            }
        }
        Ok(reaped)
    }

    // automation_run_events

    pub async fn append_event(
        &self,
        run_id: Uuid,
        kind: EventKind,
        payload: Option<&serde_json::Value>,
    ) -> RepositoryResult<i64> {
        let ts = Self::now_string();
        let payload_str = payload.map(serde_json::to_string).transpose()?;
        let res = sqlx::query(
            "INSERT INTO automation_run_events (run_id, ts, kind, payload) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(&ts)
        .bind(kind.as_str())
        .bind(&payload_str)
        .execute(&self.pool)
        .await?;
        Ok(res.last_insert_rowid())
    }

    pub async fn list_events_for_run(
        &self,
        run_id: Uuid,
    ) -> RepositoryResult<Vec<DbAutomationRunEvent>> {
        let rows = sqlx::query(
            "SELECT id, run_id, ts, kind, payload \
             FROM automation_run_events WHERE run_id = ? ORDER BY ts ASC, id ASC",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_db_event).collect()
    }
}
