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

/// Create response: for a webhook trigger, `webhook_token` carries the raw token
/// ONCE (only its hash is stored). Present POSTs to `POST /webhooks/automations`
/// with `Authorization: Bearer <token>`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CreatedTriggerResponse {
    #[serde(flatten)]
    pub trigger: TriggerResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_token: Option<String>,
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
) -> Result<(StatusCode, Json<CreatedTriggerResponse>), ApiError> {
    let automation = require_owned_automation(&state, &auth, automation_id).await?;

    // Cron: validate the expression + compute the first fire instant. Webhook:
    // issue a token (client never provides it) and store only its hash.
    let (mut token_hash, mut webhook_token) = (None, None);
    let next_fire_at = match &payload.spec {
        TriggerSpec::Cron { expr, tz } => {
            let tz_name = tz.as_deref().unwrap_or(default_tz_name());
            let fire = next_fire_after(expr, tz_name, Utc::now())
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            // Schedule only while both the trigger and its automation are enabled;
            // re-enabling the automation recomputes it (see update_automation).
            (payload.enabled && automation.enabled).then_some(fire)
        }
        TriggerSpec::Webhook {} => {
            let (token, hash) = crate::state::AutomationsState::new_webhook_token();
            token_hash = Some(hash);
            webhook_token = Some(token);
            None
        }
    };

    let trigger = state
        .automations
        .create_trigger(automation_id, &payload.spec, payload.enabled, next_fire_at, token_hash)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedTriggerResponse {
            trigger: TriggerResponse::from_db(trigger)?,
            webhook_token,
        }),
    ))
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTriggerRequest {
    /// New spec (same kind — kind is immutable). Omit to keep the current spec.
    #[serde(default)]
    pub spec: Option<TriggerSpec>,
    /// New enabled state. Omit to keep it.
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Update a trigger's spec and/or enabled state. Kind is immutable; a cron
/// change recomputes `next_fire_at` (parked when the trigger or automation is off).
pub(super) async fn update_trigger(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path((automation_id, trigger_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<UpdateTriggerRequest>,
) -> Result<Json<TriggerResponse>, ApiError> {
    require_owned_automation(&state, &auth, automation_id).await?;
    let trigger = state
        .automations
        .get_trigger(trigger_id)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "trigger not found"))?;
    if trigger.automation_id != automation_id {
        return Err(err(StatusCode::NOT_FOUND, "trigger not found"));
    }
    let updated = state
        .automations
        .update_trigger(trigger_id, payload.spec.as_ref(), payload.enabled)
        .await?;
    Ok(Json(TriggerResponse::from_db(updated)?))
}

/// Public webhook receiver (no auth middleware): fires the automation whose
/// webhook trigger matches the `Authorization: Bearer <token>` header. The token
/// hashes to a globally-unique `webhook_token_hash`, so it alone identifies the
/// trigger. The request body is accepted but not yet used (body → render input
/// arrives with the event infrastructure). A generic 401 masks unknown tokens.
pub(super) async fn fire_webhook_trigger(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    _body: axum::body::Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "invalid token"))?;
    let hash = crate::state::AutomationsState::webhook_token_hash(token);
    let trigger = state
        .automations
        .find_trigger_by_webhook_token_hash(&hash)
        .await?
        .filter(|t| t.enabled)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "invalid token"))?;
    let automation = state
        .automations
        .get_automation(trigger.automation_id)
        .await?
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "trigger's automation missing"))?;
    match state.automations.create_webhook_run(&automation, trigger.id).await? {
        Some(run) => Ok((StatusCode::ACCEPTED, Json(serde_json::json!({ "run_id": run.id })))),
        None => Err(err(StatusCode::CONFLICT, "automation is disabled")),
    }
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
    /// With `to`, restrict to runs whose `scheduled_for` is in `[from, to)`
    /// (workspace-scoped) — the calendar/timeline window.
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// `GET /automation-runs?automation_id=&workspace_id=`
pub(super) async fn list_runs(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<ListRunsQuery>,
) -> Result<Json<RunListResponse>, ApiError> {
    let runs = if let (Some(from), Some(to)) = (q.from, q.to) {
        let ws = q.workspace_id.unwrap_or(auth.id);
        require_owned_workspace(&state, &auth, ws).await?;
        if to <= from {
            return Err(err(StatusCode::BAD_REQUEST, "`to` must be after `from`"));
        }
        state
            .automations
            .list_runs_in_window(ws, from, to, RUN_WINDOW_MAX)
            .await?
    } else if let Some(aid) = q.automation_id {
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

const RUN_WINDOW_MAX: i64 = 500;
const OCCURRENCE_PER_TRIGGER_MAX: usize = 500;
const OCCURRENCE_WINDOW_MAX_DAYS: i64 = 366;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OccurrencesQuery {
    pub workspace_id: Option<Uuid>,
    /// Window start (RFC3339); defaults to now.
    pub from: Option<DateTime<Utc>>,
    /// Window end (RFC3339, exclusive); defaults to `from` + 31 days.
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OccurrenceResponse {
    pub trigger_id: Uuid,
    pub automation_id: Uuid,
    pub automation_name: String,
    pub fire_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OccurrenceListResponse {
    pub items: Vec<OccurrenceResponse>,
    /// True if any trigger hit the per-trigger expansion cap.
    pub truncated: bool,
}

/// Predicted cron fire instants across a workspace's enabled automations within
/// `[from, to)` — the schedule/calendar preview.
pub(super) async fn list_occurrences(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<OccurrencesQuery>,
) -> Result<Json<OccurrenceListResponse>, ApiError> {
    let ws = q.workspace_id.unwrap_or(auth.id);
    require_owned_workspace(&state, &auth, ws).await?;
    let from = q.from.unwrap_or_else(Utc::now);
    let to = q.to.unwrap_or_else(|| from + chrono::Duration::days(31));
    if to <= from {
        return Err(err(StatusCode::BAD_REQUEST, "`to` must be after `from`"));
    }
    if to - from > chrono::Duration::days(OCCURRENCE_WINDOW_MAX_DAYS) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("window too wide (max {OCCURRENCE_WINDOW_MAX_DAYS} days)"),
        ));
    }

    let triggers = state
        .automations
        .list_enabled_cron_triggers_by_workspace(ws)
        .await?;
    let mut items: Vec<OccurrenceResponse> = Vec::new();
    let mut truncated = false;
    for (trigger, automation_name) in triggers {
        let TriggerSpec::Cron { expr, tz } = TriggerSpec::from_db(trigger.kind, &trigger.spec_json)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("spec decode: {e}")))?
        else {
            continue; // query already filters kind = 'cron'
        };
        let tz_name = tz.as_deref().unwrap_or(default_tz_name());
        let (fires, trig_truncated) =
            match crate::cron::occurrences_between(&expr, tz_name, from, to, OCCURRENCE_PER_TRIGGER_MAX) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(trigger = %trigger.id, "occurrences expand skipped: {e}");
                    continue;
                }
            };
        truncated |= trig_truncated;
        items.extend(fires.into_iter().map(|fire_at| OccurrenceResponse {
            trigger_id: trigger.id,
            automation_id: trigger.automation_id,
            automation_name: automation_name.clone(),
            fire_at,
            tz: tz.clone(),
        }));
    }
    items.sort_by_key(|o| o.fire_at);
    Ok(Json(OccurrenceListResponse { items, truncated }))
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

/// Delete a run and its audit trail. Removing the run's session cascades the run
/// row and its logs in one atomic DELETE; a non-terminal run is refused (409) so
/// this can't race the worker mid-drive — cancel it first.
pub(super) async fn delete_run(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(run_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let run = require_owned_run(&state, &auth, run_id).await?;
    if matches!(run.status, RunStatus::Queued | RunStatus::Running) {
        return Err(err(
            StatusCode::CONFLICT,
            "run is not terminal; cancel it before deleting",
        ));
    }
    state.delete_session(run.session_id).await?;
    Ok(StatusCode::NO_CONTENT)
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
