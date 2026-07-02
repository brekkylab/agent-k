use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    state::{Agent, AppState, Session, Workspace},
};

use super::error::{ApiError, err};

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkspaceResponse {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Workspace> for WorkspaceResponse {
    fn from(w: Workspace) -> Self {
        Self {
            id: w.id,
            title: w.title,
            created_at: w.created_at,
            updated_at: w.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkspaceListResponse {
    pub items: Vec<WorkspaceResponse>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateWorkspaceRequest {
    pub title: Option<String>,
}

/// Fetch a workspace the caller may access (see
/// [`WorkspacesState::get_for_user`](crate::state::WorkspacesState::get_for_user));
/// a workspace the caller can't reach is reported as `404` so its existence
/// can't be probed. Reused by the other resource routers.
pub(super) async fn require_owned_workspace(
    state: &AppState,
    auth: &AuthUser,
    wid: Uuid,
) -> Result<Workspace, ApiError> {
    state
        .workspaces
        .get_for_user(auth.id, wid)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "workspace not found"))
}

/// Fetch an agent, ensuring the caller may access the workspace it lives in.
pub(super) async fn require_owned_agent(
    state: &AppState,
    auth: &AuthUser,
    aid: Uuid,
) -> Result<Agent, ApiError> {
    let agent = state
        .agents
        .get(aid)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "agent not found"))?;
    require_owned_workspace(state, auth, agent.workspace_id).await?;
    Ok(agent)
}

/// Fetch a session, ensuring the caller may access the workspace it lives in.
pub(super) async fn require_owned_session(
    state: &AppState,
    auth: &AuthUser,
    sid: Uuid,
) -> Result<Session, ApiError> {
    let session = state
        .sessions
        .get(sid)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "session not found"))?;
    require_owned_workspace(state, auth, session.workspace_id).await?;
    Ok(session)
}

pub(super) async fn list_workspaces(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<WorkspaceListResponse>, ApiError> {
    let items = state
        .workspaces
        .get(auth.id)
        .await?
        .into_iter()
        .map(WorkspaceResponse::from)
        .collect();
    Ok(Json(WorkspaceListResponse { items }))
}

pub(super) async fn get_workspace(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let workspace = require_owned_workspace(&state, &auth, id).await?;
    Ok(Json(WorkspaceResponse::from(workspace)))
}

pub(super) async fn update_workspace(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let existing = require_owned_workspace(&state, &auth, id).await?;
    let updated = match payload.title {
        Some(t) => existing.with_title(t).with_updated_at(),
        None => existing,
    };
    state.workspaces.upsert(updated.clone()).await?;
    Ok(Json(WorkspaceResponse::from(updated)))
}

pub(super) async fn delete_workspace(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_owned_workspace(&state, &auth, id).await?;
    // The default workspace (id == user id) is not user-deletable; it is only
    // removed when the account itself is deleted.
    if id == auth.id {
        return Err(err(
            StatusCode::FORBIDDEN,
            "cannot delete your default workspace",
        ));
    }
    state.workspaces.remove(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /me/workspace` — the caller's default workspace (id == user id).
pub(super) async fn get_my_workspace(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let workspace = require_owned_workspace(&state, &auth, auth.id).await?;
    Ok(Json(WorkspaceResponse::from(workspace)))
}

/// `PATCH /me/workspace` — update the caller's default workspace.
pub(super) async fn update_my_workspace(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Json(payload): Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let existing = require_owned_workspace(&state, &auth, auth.id).await?;
    let updated = match payload.title {
        Some(t) => existing.with_title(t).with_updated_at(),
        None => existing,
    };
    state.workspaces.upsert(updated.clone()).await?;
    Ok(Json(WorkspaceResponse::from(updated)))
}
