use std::sync::Arc;

use aide::axum::{ApiRouter, routing::get};
use ailoy::agent::AgentSpec;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::{AppState, Session, StateError};

use super::error::{ApiError, err};

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: Option<String>,
    pub spec: AgentSpec,
    pub runenv: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Session> for SessionResponse {
    fn from(s: Session) -> Self {
        Self {
            id: s.id,
            project_id: s.project_id,
            title: s.title,
            spec: s.spec,
            runenv: s.runenv,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionListResponse {
    pub items: Vec<SessionResponse>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {
    pub project_id: Uuid,
    pub title: Option<String>,
    pub spec: AgentSpec,
}

pub fn get_session_router(state: Arc<AppState>) -> ApiRouter {
    ApiRouter::new()
        .api_route(
            "/sessions",
            get(list_sessions).post(create_session),
        )
        .api_route(
            "/sessions/{id}",
            get(get_session).delete(delete_session),
        )
        .with_state(state)
}

async fn list_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let sessions = state.sessions.list().await?;
    Ok(Json(SessionListResponse {
        items: sessions.into_iter().map(SessionResponse::from).collect(),
    }))
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError> {
    if state.projects.get(payload.project_id).await?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "project not found"));
    }

    let mut session = Session::new(payload.project_id, payload.spec);
    if let Some(t) = payload.title {
        session = session.with_title(t);
    }
    state.sessions.insert(session.clone(), None).await?;
    Ok((StatusCode::CREATED, Json(SessionResponse::from(session))))
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionResponse>, ApiError> {
    let session = state
        .sessions
        .get(id)
        .await?
        .ok_or(StateError::NotFound)?;
    Ok(Json(SessionResponse::from(session)))
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.sessions.remove(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
