use std::sync::Arc;

use agent_k::agents::{get_coworker_agent_spec, get_deep_research_agent_spec};
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
    pub agent_id: Option<Uuid>,
    pub title: Option<String>,
    pub spec: AgentSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Session> for SessionResponse {
    fn from(s: Session) -> Self {
        Self {
            id: s.id,
            project_id: s.project_id,
            agent_id: s.agent_id,
            title: s.title,
            spec: s.spec,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionListResponse {
    pub items: Vec<SessionResponse>,
}

/// Identity passed as `name` to agent-k's spec builders. Per-agent identity is
/// not yet a configurable concept in v2.
const SESSION_AGENT_NAME: &str = "agent-k";

const DEFAULT_MODEL_COWORKER: &str = "anthropic/claude-sonnet-4-5";
const DEFAULT_MODEL_DEEP_RESEARCH: &str = "anthropic/claude-sonnet-4-5";

/// Selects which agent-k preset builds the [`AgentSpec`] when creating a
/// session. Variants correspond 1:1 to the `get_*_agent_spec` family in
/// [`agent_k::agents`]; [`build_spec`] is the dispatch.
// TODO: add `Speedwagon` variant once the knowledge-base store wiring is ready.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    Coworker,
    DeepResearch,
}

fn build_spec(agent_type: AgentType, model: Option<&str>) -> AgentSpec {
    match agent_type {
        AgentType::Coworker => {
            get_coworker_agent_spec(
                SESSION_AGENT_NAME,
                model.unwrap_or(DEFAULT_MODEL_COWORKER),
                true,
            )
        }
        AgentType::DeepResearch => get_deep_research_agent_spec(
            SESSION_AGENT_NAME,
            model.unwrap_or(DEFAULT_MODEL_DEEP_RESEARCH),
        ),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {
    pub project_id: Uuid,
    pub title: Option<String>,
    /// Create the session from a stored, project-scoped agent. When set, the
    /// agent's spec is copied into the session and `agent_type`/`model` are
    /// ignored. Mutually exclusive with `agent_type`; exactly one is required.
    #[serde(default)]
    pub agent_id: Option<Uuid>,
    /// Build the session's spec from a preset. Ignored when `agent_id` is set.
    #[serde(default)]
    pub agent_type: Option<AgentType>,
    /// Override the agent-type's default model. `None` falls back to the
    /// per-type default in [`build_spec`].
    #[serde(default)]
    pub model: Option<String>,
}

pub(super) async fn list_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let sessions = state.sessions.list().await?;
    Ok(Json(SessionListResponse {
        items: sessions.into_iter().map(SessionResponse::from).collect(),
    }))
}

pub(super) async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError> {
    if state.projects.get(payload.project_id).await?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "project not found"));
    }

    // Source the spec either from a stored agent or from a preset. Exactly one
    // of `agent_id` / `agent_type` must be supplied.
    let (spec, agent_id) = match (payload.agent_id, payload.agent_type) {
        (Some(_), Some(_)) => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "provide exactly one of agent_id or agent_type",
            ));
        }
        (Some(agent_id), None) => {
            let agent = state
                .agents
                .get(agent_id)
                .await?
                .ok_or_else(|| err(StatusCode::NOT_FOUND, "agent not found"))?;
            if agent.project_id != payload.project_id {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "agent does not belong to the given project",
                ));
            }
            if !agent.active {
                return Err(err(StatusCode::CONFLICT, "agent is not active"));
            }
            (agent.spec, Some(agent_id))
        }
        (None, Some(agent_type)) => (build_spec(agent_type, payload.model.as_deref()), None),
        (None, None) => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "provide exactly one of agent_id or agent_type",
            ));
        }
    };

    let mut session = Session::new(payload.project_id, spec);
    if let Some(aid) = agent_id {
        session = session.with_agent_id(aid);
    }
    if let Some(t) = payload.title {
        session = session.with_title(t);
    }
    state.sessions.insert(session.clone(), None).await?;
    Ok((StatusCode::CREATED, Json(SessionResponse::from(session))))
}

pub(super) async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionResponse>, ApiError> {
    let session = state.sessions.get(id).await?.ok_or(StateError::NotFound)?;
    Ok(Json(SessionResponse::from(session)))
}

pub(super) async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.sessions.remove(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
