//! Automation HTTP surface: automations CRUD, cron triggers, runs, run events.
//! Ownership is workspace-scoped (an automation belongs to a workspace the
//! caller owns); missing and foreign resources both report `404`.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    cron::{default_tz_name, next_fire_after},
    state::{
        AppState, Automation, AutomationRun, AutomationTrigger, RunLogKind, RunLog, RunStatus,
        TriggerKind, TriggerSpec,
    },
};

use super::{
    error::{ApiError, err},
    workspace::require_owned_workspace,
};

// ── DTOs ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct AutomationResponse {
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

impl From<Automation> for AutomationResponse {
    fn from(a: Automation) -> Self {
        Self {
            id: a.id,
            workspace_id: a.workspace_id,
            name: a.name,
            description: a.description,
            prompt: a.prompt,
            agent_type: a.agent_type,
            model: a.model,
            enabled: a.enabled,
            created_by: a.created_by,
            created_at: a.created_at,
            updated_at: a.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AutomationListResponse {
    pub items: Vec<AutomationResponse>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAutomationRequest {
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub prompt: String,
    /// `coworker` (default) or `deep_research`.
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
    pub prompt: Option<String>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub enabled: Option<bool>,
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
    fn from_db(t: AutomationTrigger) -> Result<Self, ApiError> {
        let spec = TriggerSpec::from_db(t.kind, &t.spec_json)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("spec decode: {e}")))?;
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

#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerListResponse {
    pub items: Vec<TriggerResponse>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTriggerRequest {
    #[serde(flatten)]
    pub spec: TriggerSpec,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunResponse {
    pub id: Uuid,
    /// `null` for ad-hoc one-time runs (no owning automation).
    pub automation_id: Option<Uuid>,
    pub trigger_id: Option<Uuid>,
    pub session_id: Uuid,
    pub workspace_id: Uuid,
    /// The rendered prompt this run executes.
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

impl From<AutomationRun> for RunResponse {
    fn from(r: AutomationRun) -> Self {
        Self {
            id: r.id,
            automation_id: r.automation_id,
            trigger_id: r.trigger_id,
            session_id: r.session_id,
            workspace_id: r.workspace_id,
            prompt: r.prompt,
            agent_type: r.agent_type,
            model: r.model,
            status: r.status,
            scheduled_for: r.scheduled_for,
            lease_until: r.lease_until,
            previous_run_id: r.previous_run_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunListResponse {
    pub items: Vec<RunResponse>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunLogResponse {
    pub id: i64,
    pub run_id: Uuid,
    pub ts: DateTime<Utc>,
    pub kind: RunLogKind,
    pub payload: Option<serde_json::Value>,
}

impl From<RunLog> for RunLogResponse {
    fn from(e: RunLog) -> Self {
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
pub struct RunLogListResponse {
    pub items: Vec<RunLogResponse>,
}

// ── ownership helpers ──────────────────────────────────────────────────────

/// Fetch an automation the caller owns (via its workspace). Missing and foreign
/// automations both report `404`.
async fn require_owned_automation(
    state: &AppState,
    auth: &AuthUser,
    id: Uuid,
) -> Result<Automation, ApiError> {
    let automation = state
        .automations
        .get_automation(id)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "automation not found"))?;
    if state
        .workspaces
        .get_for_user(auth.id, automation.workspace_id)
        .await?
        .is_none()
    {
        return Err(err(StatusCode::NOT_FOUND, "automation not found"));
    }
    Ok(automation)
}

/// An owned run, addressed directly by id. Runs are a top-level resource
/// (independent of automations), so ownership is via the run's workspace —
/// this works for ad-hoc runs and automation-linked runs alike. Missing and
/// foreign runs both report `404`.
async fn require_owned_run(
    state: &AppState,
    auth: &AuthUser,
    run_id: Uuid,
) -> Result<AutomationRun, ApiError> {
    let run = state
        .automations
        .get_run(run_id)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "run not found"))?;
    if state
        .workspaces
        .get_for_user(auth.id, run.workspace_id)
        .await?
        .is_none()
    {
        return Err(err(StatusCode::NOT_FOUND, "run not found"));
    }
    Ok(run)
}

// ── automations ────────────────────────────────────────────────────────────

pub(super) async fn create_automation(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Json(payload): Json<CreateAutomationRequest>,
) -> Result<(StatusCode, Json<AutomationResponse>), ApiError> {
    require_owned_workspace(&state, &auth, payload.workspace_id).await?;
    if payload.name.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name must not be empty"));
    }
    if payload.prompt.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "prompt must not be empty"));
    }
    let automation = state
        .automations
        .create_automation(
            payload.workspace_id,
            payload.name,
            payload.description,
            payload.prompt,
            payload.agent_type.unwrap_or_else(|| "coworker".to_string()),
            payload.model,
            auth.id,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(automation.into())))
}

pub(super) async fn list_automations(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<AutomationListResponse>, ApiError> {
    // Default workspace id == user id (see auth bootstrap).
    let automations = state.automations.list_automations_by_workspace(auth.id).await?;
    Ok(Json(AutomationListResponse {
        items: automations.into_iter().map(AutomationResponse::from).collect(),
    }))
}

pub(super) async fn get_automation(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<AutomationResponse>, ApiError> {
    let automation = require_owned_automation(&state, &auth, id).await?;
    Ok(Json(automation.into()))
}

pub(super) async fn update_automation(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateAutomationRequest>,
) -> Result<Json<AutomationResponse>, ApiError> {
    require_owned_automation(&state, &auth, id).await?;
    if let Some(ref name) = payload.name
        && name.trim().is_empty()
    {
        return Err(err(StatusCode::BAD_REQUEST, "name must not be empty"));
    }
    let updated = state
        .automations
        .update_automation(
            id,
            payload.name,
            payload.description.map(Some),
            payload.prompt,
            payload.agent_type,
            payload.model.map(Some),
            payload.enabled,
        )
        .await?;
    Ok(Json(updated.into()))
}

pub(super) async fn delete_automation(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_owned_automation(&state, &auth, id).await?;
    state.automations.delete_automation(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── triggers ───────────────────────────────────────────────────────────────

pub(super) async fn create_trigger(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
    Json(payload): Json<CreateTriggerRequest>,
) -> Result<(StatusCode, Json<TriggerResponse>), ApiError> {
    require_owned_automation(&state, &auth, automation_id).await?;

    // Validate the cron expression up front + compute the first fire instant.
    let next_fire_at = match &payload.spec {
        TriggerSpec::Cron { expr, tz } => {
            let tz_name = tz.as_deref().unwrap_or(default_tz_name());
            let fire = next_fire_after(expr, tz_name, Utc::now())
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            payload.enabled.then_some(fire)
        }
    };

    let trigger = state
        .automations
        .create_trigger(automation_id, &payload.spec, payload.enabled, next_fire_at)
        .await?;
    Ok((StatusCode::CREATED, Json(TriggerResponse::from_db(trigger)?)))
}

pub(super) async fn list_triggers(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
) -> Result<Json<TriggerListResponse>, ApiError> {
    require_owned_automation(&state, &auth, automation_id).await?;
    let triggers = state.automations.list_triggers(automation_id).await?;
    let items = triggers
        .into_iter()
        .map(TriggerResponse::from_db)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(TriggerListResponse { items }))
}

pub(super) async fn delete_trigger(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path((automation_id, trigger_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_owned_automation(&state, &auth, automation_id).await?;
    let trigger = state
        .automations
        .get_trigger(trigger_id)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "trigger not found"))?;
    if trigger.automation_id != automation_id {
        return Err(err(StatusCode::NOT_FOUND, "trigger not found"));
    }
    state.automations.delete_trigger(trigger_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── runs (top-level resource: /automation-runs) ─────────────────────────────

/// Create body: `automation_id` runs an existing automation; otherwise
/// `workspace_id` + `prompt` make an ad-hoc run. `scheduled_for` defers pickup.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CreateRunRequest {
    pub automation_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
    pub prompt: Option<String>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub scheduled_for: Option<DateTime<Utc>>,
    pub title: Option<String>,
}

/// `POST /automation-runs`
pub(super) async fn create_run(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Json(payload): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<RunResponse>), ApiError> {
    let (at, source) = match payload.scheduled_for {
        Some(t) => (t, "one_time"),
        None => (Utc::now(), "manual"),
    };
    let run = if let Some(aid) = payload.automation_id {
        let automation = require_owned_automation(&state, &auth, aid).await?;
        if !automation.enabled {
            return Err(err(StatusCode::CONFLICT, "automation is disabled"));
        }
        state
            .automations
            .create_scheduled_run(&automation, at, source)
            .await?
    } else {
        let ws = payload.workspace_id.ok_or_else(|| {
            err(StatusCode::BAD_REQUEST, "automation_id or workspace_id is required")
        })?;
        let prompt = payload
            .prompt
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "prompt is required for ad-hoc runs"))?;
        require_owned_workspace(&state, &auth, ws).await?;
        if prompt.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "prompt must not be empty"));
        }
        let title = payload.title.unwrap_or_else(|| prompt.chars().take(60).collect());
        state
            .automations
            .create_adhoc_run(
                ws,
                title,
                prompt,
                payload.agent_type.unwrap_or_else(|| "coworker".to_string()),
                payload.model,
                at,
                auth.id,
            )
            .await?
    };
    Ok((StatusCode::CREATED, Json(run.into())))
}

/// `GET /automation-runs` filter: by automation, else by workspace.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct ListRunsQuery {
    pub automation_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
}

/// `GET /automation-runs?automation_id=&workspace_id=`
pub(super) async fn list_runs(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<ListRunsQuery>,
) -> Result<Json<RunListResponse>, ApiError> {
    let runs = if let Some(aid) = q.automation_id {
        require_owned_automation(&state, &auth, aid).await?;
        state.automations.list_runs(aid).await?
    } else {
        let ws = q.workspace_id.unwrap_or(auth.id);
        require_owned_workspace(&state, &auth, ws).await?;
        state.automations.list_runs_by_workspace(ws).await?
    };
    Ok(Json(RunListResponse {
        items: runs.into_iter().map(RunResponse::from).collect(),
    }))
}

/// `GET /automation-runs/{id}`
pub(super) async fn get_run(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<RunResponse>, ApiError> {
    let run = require_owned_run(&state, &auth, run_id).await?;
    Ok(Json(run.into()))
}

/// `POST /automation-runs/{id}/cancel`
pub(super) async fn cancel_run(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(run_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_owned_run(&state, &auth, run_id).await?;
    let payload = serde_json::json!({ "source": "manual", "by": auth.id.to_string() });
    let cancelled = state.automations.cancel_run(run_id, &payload).await?;
    if !cancelled {
        return Err(err(StatusCode::CONFLICT, "run is already terminal"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /automation-runs/{id}/events` — audit event log.
pub(super) async fn list_run_logs(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<RunLogListResponse>, ApiError> {
    require_owned_run(&state, &auth, run_id).await?;
    let events = state.automations.list_logs_for_run(run_id).await?;
    Ok(Json(RunLogListResponse {
        items: events.into_iter().map(RunLogResponse::from).collect(),
    }))
}
