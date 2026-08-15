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

/// Fetch an agent the caller owns. Missing and foreign agents return the same
/// `404` (status and message).
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
    if state
        .workspaces
        .get_for_user(auth.id, agent.workspace_id)
        .await?
        .is_none()
    {
        return Err(err(StatusCode::NOT_FOUND, "agent not found"));
    }
    Ok(agent)
}

/// Fetch a session the caller owns. Missing and foreign sessions return the same
/// `404` (status and message).
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
    if state
        .workspaces
        .get_for_user(auth.id, session.workspace_id)
        .await?
        .is_none()
    {
        return Err(err(StatusCode::NOT_FOUND, "session not found"));
    }
    Ok(session)
}

/// `GET /workspaces` — list the caller's workspaces. Not routed yet: with a
/// single workspace per user it only ever returns the default, which
/// `/me/workspace` already serves. Re-wire (as a real list) with multi-workspace.
#[allow(dead_code)]
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

/// `DELETE /workspaces/{id}` — remove a non-default workspace the caller owns.
/// Not routed yet: non-default workspaces can't be created, so this is currently
/// unreachable. Kept (with the default-protection guard) for when multiple
/// workspaces per user are supported.
#[allow(dead_code)]
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
    state.delete_workspace(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /me/workspace` — the caller's default workspace (id == user id).
/// Thin alias that delegates to [`get_workspace`] with the caller's id, so the
/// two share one implementation.
pub(super) async fn get_my_workspace(
    state: State<Arc<AppState>>,
    auth: Extension<AuthUser>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let id = auth.id;
    get_workspace(state, auth, Path(id)).await
}

/// `PATCH /me/workspace` — update the caller's default workspace. Delegates to
/// [`update_workspace`] with the caller's id.
pub(super) async fn update_my_workspace(
    state: State<Arc<AppState>>,
    auth: Extension<AuthUser>,
    payload: Json<UpdateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, ApiError> {
    let id = auth.id;
    update_workspace(state, auth, Path(id), payload).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use ailoy::agent::AgentSpec;
    use axum::Json;

    use crate::auth::{JwtConfig, Role};
    use crate::state::NewUser;

    async fn test_state() -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db_url = format!("sqlite://{}/test.db", tmp.path().display());
        let jwt = JwtConfig::new("test-secret", 3600);
        let state = AppState::new(
            &db_url,
            tmp.path().to_path_buf(),
            jwt,
            // No provider OAuth apps configured: these tests don't create mounts.
            Default::default(),
            Default::default(),
        )
        .await
        .unwrap();
        (state, tmp)
    }

    async fn seed_user(state: &AppState, username: &str) -> AuthUser {
        let user = state
            .users
            .create(NewUser {
                id: Uuid::new_v4(),
                username: username.to_string(),
                password_hash: "x".into(),
                role: Role::User,
                display_name: None,
                is_active: true,
                preferred_language: "en".into(),
            })
            .await
            .unwrap();
        state.workspaces.create_default(&user).await.unwrap();
        AuthUser {
            id: user.id,
            username: user.username,
            role: user.role,
        }
    }

    // Missing and foreign ids must yield the identical 404 (status and body).
    #[tokio::test]
    async fn require_owned_agent_hides_cross_tenant_existence() {
        let (state, _tmp) = test_state().await;
        let alice = seed_user(&state, "alice").await;
        let bob = seed_user(&state, "bob").await;

        let agent = Agent::new(
            alice.id,
            "researcher",
            AgentSpec::new("anthropic/claude-sonnet-4-5"),
        );
        let agent_id = agent.id;
        state.agents.upsert(agent).await.unwrap();

        let (fstatus, Json(fbody)) =
            require_owned_agent(&state, &bob, agent_id).await.unwrap_err();
        let (mstatus, Json(mbody)) = require_owned_agent(&state, &bob, Uuid::new_v4())
            .await
            .unwrap_err();
        assert_eq!(fstatus, StatusCode::NOT_FOUND);
        assert_eq!(fstatus, mstatus);
        assert_eq!(fbody.error, mbody.error);

        // The owner still reaches her own agent.
        assert!(require_owned_agent(&state, &alice, agent_id).await.is_ok());
    }

    #[tokio::test]
    async fn require_owned_session_hides_cross_tenant_existence() {
        let (state, _tmp) = test_state().await;
        let alice = seed_user(&state, "alice").await;
        let bob = seed_user(&state, "bob").await;

        let session = Session::new(alice.id, AgentSpec::new("anthropic/claude-sonnet-4-5"));
        let sid = session.id;
        state.sessions.insert(session, None).await.unwrap();

        let (fstatus, Json(fbody)) =
            require_owned_session(&state, &bob, sid).await.unwrap_err();
        let (mstatus, Json(mbody)) = require_owned_session(&state, &bob, Uuid::new_v4())
            .await
            .unwrap_err();
        assert_eq!(fstatus, StatusCode::NOT_FOUND);
        assert_eq!(fstatus, mstatus);
        assert_eq!(fbody.error, mbody.error);

        // The owner still reaches her own session.
        assert!(require_owned_session(&state, &alice, sid).await.is_ok());
    }
}
