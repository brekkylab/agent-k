use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiResult, AppError},
    model::{
        AutomationListResponse, AutomationResponse, CreateAutomationRequest, CreateTriggerRequest,
        CreatedTriggerResponse, EventListResponse, EventResponse, RunListResponse, RunResponse,
        TriggerListResponse, TriggerResponse, TriggerSpec, UpdateAutomationRequest,
        UpdateTriggerRequest,
    },
    repository::{DbAutomation, RepositoryError},
    state::AppState,
};

// ── automations ──────────────────────────────────────────────────────────────

/// POST /automations — body includes `project_id`.
pub async fn create_automation(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Json(payload): Json<CreateAutomationRequest>,
) -> ApiResult<(StatusCode, Json<AutomationResponse>)> {
    let project_id = payload.project_id;
    require_member(&state, auth_user.id, project_id).await?;
    if payload.name.trim().is_empty() {
        return Err(AppError::bad_request("name must not be empty"));
    }

    let automation = state
        .repository
        .create_automation(
            project_id,
            payload.name,
            payload.description,
            payload.prompts,
            auth_user.id,
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    tracing::info!(id = %automation.id, project = %project_id, "automation created");
    Ok((StatusCode::CREATED, Json(automation.into())))
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ListAutomationsQuery {
    pub project_id: Option<Uuid>,
}

/// GET /automations?project_id=... — `project_id` optional; omit for all
/// automations across the user's accessible projects.
pub async fn list_automations(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Query(q): Query<ListAutomationsQuery>,
) -> ApiResult<Json<AutomationListResponse>> {
    let automations = match q.project_id {
        Some(project_id) => {
            require_member(&state, auth_user.id, project_id).await?;
            state
                .repository
                .list_automations_in_project(project_id)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?
        }
        None => state
            .repository
            .list_automations_for_user(auth_user.id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    };
    Ok(Json(AutomationListResponse {
        items: automations.into_iter().map(AutomationResponse::from).collect(),
    }))
}

/// GET /automations/{automation_id}
pub async fn get_automation(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
) -> ApiResult<Json<AutomationResponse>> {
    let automation = require_automation_access(&state, auth_user.id, automation_id).await?;
    Ok(Json(automation.into()))
}

/// PATCH /automations/{automation_id}
pub async fn update_automation(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
    Json(payload): Json<UpdateAutomationRequest>,
) -> ApiResult<Json<AutomationResponse>> {
    require_automation_access(&state, auth_user.id, automation_id).await?;

    if let Some(ref name) = payload.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("name must not be empty"));
        }
    }

    let updated = state
        .repository
        .update_automation(
            automation_id,
            payload.name,
            payload.description.map(Some),
            payload.prompts,
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(updated.into()))
}

/// DELETE /automations/{automation_id} — CASCADE removes triggers/runs/events.
pub async fn delete_automation(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    require_automation_access(&state, auth_user.id, automation_id).await?;
    state
        .repository
        .delete_automation(automation_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    tracing::info!(id = %automation_id, "automation deleted");
    Ok(StatusCode::NO_CONTENT)
}

// ── triggers ─────────────────────────────────────────────────────────────────

/// POST /automations/{automation_id}/triggers — webhook variant returns
/// a one-time plaintext token; DB stores only its SHA-256 hash.
pub async fn create_trigger(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
    Json(spec): Json<CreateTriggerRequest>,
) -> ApiResult<(StatusCode, Json<CreatedTriggerResponse>)> {
    require_automation_access(&state, auth_user.id, automation_id).await?;

    let (token_hash, plaintext) = if matches!(spec, TriggerSpec::Webhook { .. }) {
        let token = generate_webhook_token();
        (Some(sha256_hex(&token)), Some(token))
    } else {
        (None, None)
    };

    let next_fire_at = if let TriggerSpec::Cron { expr, tz } = &spec {
        let default_tz = crate::cron::default_tz_name();
        let tz_name = tz.as_deref().unwrap_or(default_tz);
        let next = crate::cron::next_fire_after(expr, tz_name, chrono::Utc::now())
            .map_err(AppError::bad_request)?;
        Some(next)
    } else {
        None
    };

    let trigger = state
        .repository
        .create_trigger(automation_id, &spec, token_hash, next_fire_at)
        .await
        .map_err(|e| match e {
            RepositoryError::UniqueViolation(_) => {
                AppError::conflict("webhook token collision; retry")
            }
            other => AppError::internal(other.to_string()),
        })?;

    let trigger_response = TriggerResponse::from_db(trigger)
        .map_err(|e| AppError::internal(format!("trigger spec decode: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(CreatedTriggerResponse {
            trigger: trigger_response,
            webhook_token: plaintext,
        }),
    ))
}

/// GET /automations/{automation_id}/triggers
pub async fn list_triggers(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
) -> ApiResult<Json<TriggerListResponse>> {
    require_automation_access(&state, auth_user.id, automation_id).await?;
    let triggers = state
        .repository
        .list_triggers_for_automation(automation_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let items = triggers
        .into_iter()
        .map(TriggerResponse::from_db)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::internal(format!("trigger spec decode: {e}")))?;

    Ok(Json(TriggerListResponse { items }))
}

/// GET /automations/{automation_id}/triggers/{trigger_id}
pub async fn get_trigger(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((automation_id, trigger_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<TriggerResponse>> {
    let (trigger, _automation) =
        require_nested_trigger_access(&state, auth_user.id, automation_id, trigger_id).await?;
    let response = TriggerResponse::from_db(trigger)
        .map_err(|e| AppError::internal(format!("trigger spec decode: {e}")))?;
    Ok(Json(response))
}

/// PATCH /automations/{automation_id}/triggers/{trigger_id}
pub async fn update_trigger(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((automation_id, trigger_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<UpdateTriggerRequest>,
) -> ApiResult<Json<TriggerResponse>> {
    let (current, _automation) =
        require_nested_trigger_access(&state, auth_user.id, automation_id, trigger_id).await?;

    // Disallow changing the trigger kind once created (would orphan webhook tokens etc.).
    if let Some(ref spec) = payload.spec {
        if spec.kind() != current.kind {
            return Err(AppError::bad_request(
                "trigger kind is immutable; delete and recreate to change kind",
            ));
        }
    }

    // Recompute next_fire_at if the cron expression / tz changed.
    let next_fire_at = match payload.spec.as_ref() {
        Some(TriggerSpec::Cron { expr, tz }) => {
            let default_tz = crate::cron::default_tz_name();
            let tz_name = tz.as_deref().unwrap_or(default_tz);
            Some(Some(
                crate::cron::next_fire_after(expr, tz_name, chrono::Utc::now())
                    .map_err(AppError::bad_request)?,
            ))
        }
        _ => None,
    };

    let updated = state
        .repository
        .update_trigger(trigger_id, payload.spec.as_ref(), payload.enabled, next_fire_at)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let response = TriggerResponse::from_db(updated)
        .map_err(|e| AppError::internal(format!("trigger spec decode: {e}")))?;
    Ok(Json(response))
}

/// DELETE /automations/{automation_id}/triggers/{trigger_id}
pub async fn delete_trigger(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((automation_id, trigger_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    require_nested_trigger_access(&state, auth_user.id, automation_id, trigger_id).await?;
    state
        .repository
        .delete_trigger(trigger_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ── runs / events (read-only) ────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields, default)]
pub struct RunListQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// POST /automations/{automation_id}/runs — manual run trigger.
/// Atomically creates a new automation session, the queued run, and the
/// triggered/queued events. The worker picks it up asynchronously.
pub async fn create_run(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<RunResponse>)> {
    let automation = require_automation_access(&state, auth_user.id, automation_id).await?;

    let triggered_payload = json!({
        "source": "manual",
        "actor_user_id": auth_user.id,
    });

    let run = state
        .repository
        .create_automation_run_with_session(
            automation_id,
            automation.project_id,
            auth_user.id,
            None,
            Utc::now(),
            None,
            Some(&triggered_payload),
            None,
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    tracing::info!(run = %run.id, automation = %automation_id, "manual run queued");
    Ok((StatusCode::CREATED, Json(run.into())))
}

/// GET /automations/{automation_id}/runs?limit=&offset=
pub async fn list_runs(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(automation_id): Path<Uuid>,
    Query(q): Query<RunListQuery>,
) -> ApiResult<Json<RunListResponse>> {
    require_automation_access(&state, auth_user.id, automation_id).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let runs = state
        .repository
        .list_runs_for_automation(automation_id, limit, offset)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(RunListResponse {
        items: runs.into_iter().map(RunResponse::from).collect(),
    }))
}

/// GET /automations/{automation_id}/runs/{run_id}
pub async fn get_run(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((automation_id, run_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<RunResponse>> {
    let run = require_nested_run_access(&state, auth_user.id, automation_id, run_id).await?;
    Ok(Json(run.into()))
}

/// GET /automations/{automation_id}/runs/{run_id}/events
pub async fn list_run_events(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((automation_id, run_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<EventListResponse>> {
    require_nested_run_access(&state, auth_user.id, automation_id, run_id).await?;
    let events = state
        .repository
        .list_events_for_run(run_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(EventListResponse {
        items: events.into_iter().map(EventResponse::from).collect(),
    }))
}

// ── helpers ──────────────────────────────────────────────────────────────────

async fn require_member(
    state: &Arc<AppState>,
    user_id: Uuid,
    project_id: Uuid,
) -> ApiResult<()> {
    let exists = state
        .repository
        .get_project(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_some();
    if !exists {
        return Err(AppError::not_found("project not found"));
    }
    let is_member = state
        .repository
        .user_in_project(user_id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_member {
        Err(AppError::forbidden("not a member of this project"))
    } else {
        Ok(())
    }
}

async fn require_automation_access(
    state: &Arc<AppState>,
    user_id: Uuid,
    automation_id: Uuid,
) -> ApiResult<DbAutomation> {
    let automation = state
        .repository
        .get_automation(automation_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("automation not found"))?;
    require_member(state, user_id, automation.project_id).await?;
    Ok(automation)
}

/// Trigger exists + belongs to `automation_id` + user has project access.
/// Path mismatch returns 404 (don't leak existence).
async fn require_nested_trigger_access(
    state: &Arc<AppState>,
    user_id: Uuid,
    automation_id: Uuid,
    trigger_id: Uuid,
) -> ApiResult<(crate::repository::DbAutomationTrigger, DbAutomation)> {
    let trigger = state
        .repository
        .get_trigger(trigger_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("trigger not found"))?;
    if trigger.automation_id != automation_id {
        return Err(AppError::not_found("trigger not found"));
    }
    let automation = require_automation_access(state, user_id, automation_id).await?;
    Ok((trigger, automation))
}

/// Run exists + belongs to `automation_id` + user has project access.
/// Path mismatch returns 404.
async fn require_nested_run_access(
    state: &Arc<AppState>,
    user_id: Uuid,
    automation_id: Uuid,
    run_id: Uuid,
) -> ApiResult<crate::repository::DbAutomationRun> {
    let run = state
        .repository
        .get_run(run_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("run not found"))?;
    if run.automation_id != automation_id {
        return Err(AppError::not_found("run not found"));
    }
    require_automation_access(state, user_id, automation_id).await?;
    Ok(run)
}

// ── webhook firing (auth-exempt route) ───────────────────────────────────────

/// POST /webhooks/automations
/// Bearer token both identifies and authenticates: its SHA-256 hash is
/// looked up against the unique partial index on
/// `automation_triggers.webhook_token_hash`. JWT middleware is bypassed.
/// An optional `Idempotency-Key` header dedupes per-trigger retries; expired
/// keys are NULL'd by the housekeeper so they may be reused after the
/// retention window.
pub async fn fire_webhook_trigger(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    _body: Bytes,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    // Generic 401 for missing header, unknown token, or trigger not found.
    let presented_token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::unauthorized("invalid token"))?;

    let presented_hash = sha256_hex(presented_token);
    let trigger = state
        .repository
        .find_trigger_by_webhook_token_hash(&presented_hash)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::unauthorized("invalid token"))?;

    if !trigger.enabled {
        return Err(AppError::conflict("trigger is disabled"));
    }

    // Spec decode purely as a defensive guard — only webhook triggers carry a
    // token hash, so reaching here means it must be Webhook variant.
    let spec = TriggerSpec::from_db(trigger.kind, &trigger.spec_json)
        .map_err(|e| AppError::internal(format!("trigger spec decode: {e}")))?;
    let TriggerSpec::Webhook {} = spec else {
        return Err(AppError::internal("webhook trigger spec mismatch"));
    };

    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // Pre-check: caller-supplied key has matched a still-valid run → replay.
    if let Some(ref key) = idempotency_key {
        if let Some(existing) = state
            .repository
            .find_webhook_run_by_idempotency_key(trigger.id, key)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
        {
            return Ok(webhook_accepted_response(&existing));
        }
    }

    let automation = state
        .repository
        .get_automation(trigger.automation_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::internal("trigger's automation missing"))?;

    let triggered_payload = json!({
        "source": "webhook",
        "trigger_id": trigger.id.to_string(),
    });

    let run = match state
        .repository
        .create_automation_run_with_session(
            automation.id,
            automation.project_id,
            automation.created_by,
            Some(trigger.id),
            Utc::now(),
            None,
            Some(&triggered_payload),
            idempotency_key.as_deref(),
        )
        .await
    {
        Ok(r) => r,
        // Race fallback: a concurrent retry with the same key inserted first
        // and our INSERT lost the UNIQUE check. Re-lookup and return that row.
        Err(RepositoryError::UniqueViolation(_)) if idempotency_key.is_some() => {
            let key = idempotency_key.as_ref().unwrap();
            let existing = state
                .repository
                .find_webhook_run_by_idempotency_key(trigger.id, key)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?
                .ok_or_else(|| {
                    AppError::internal("idempotency race: UNIQUE violation but no row found")
                })?;
            return Ok(webhook_accepted_response(&existing));
        }
        Err(e) => return Err(AppError::internal(e.to_string())),
    };

    tracing::info!(trigger = %trigger.id, run = %run.id, "webhook trigger fired");

    Ok(webhook_accepted_response(&run))
}

fn webhook_accepted_response(
    run: &crate::repository::DbAutomationRun,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "run_id": run.id.to_string(),
            "session_id": run.session_id.to_string(),
            "status": "queued",
        })),
    )
}

fn generate_webhook_token() -> String {
    // 256 bits of entropy from two UUID v4s (random-source backed by OS RNG).
    let a = Uuid::new_v4().simple().to_string();
    let b = Uuid::new_v4().simple().to_string();
    format!("{a}{b}")
}

fn sha256_hex(s: impl AsRef<[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_ref());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_vector() {
        // sha256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        assert_eq!(
            sha256_hex("hello world"),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn generated_token_is_unique_per_call() {
        let a = generate_webhook_token();
        let b = generate_webhook_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }
}
