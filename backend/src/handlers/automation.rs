use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
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

    let trigger = state
        .repository
        .create_trigger(automation_id, &spec, token_hash, None)
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

    let updated = state
        .repository
        .update_trigger(trigger_id, payload.spec.as_ref(), payload.enabled, None)
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

fn generate_webhook_token() -> String {
    // 256 bits of entropy from two UUID v4s (random-source backed by OS RNG).
    let a = Uuid::new_v4().simple().to_string();
    let b = Uuid::new_v4().simple().to_string();
    format!("{a}{b}")
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
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
