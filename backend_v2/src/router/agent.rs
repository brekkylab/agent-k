use std::sync::Arc;

use ailoy::agent::AgentSpec;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::{Agent, AppState, StateError};

use super::error::{ApiError, err};

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub active: bool,
    pub runenv: bool,
    pub spec: AgentSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Agent> for AgentResponse {
    fn from(a: Agent) -> Self {
        Self {
            id: a.id,
            project_id: a.project_id,
            name: a.name,
            description: a.description,
            active: a.active,
            runenv: a.runenv,
            spec: a.spec,
            created_at: a.created_at,
            updated_at: a.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentListResponse {
    pub items: Vec<AgentResponse>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAgentsQuery {
    /// Restrict the listing to a single project. Omit to list every agent.
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentRequest {
    pub project_id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Full agent definition (model, instruction, tools, sub-agents, …).
    pub spec: AgentSpec,
    /// Defaults to `true` when omitted.
    #[serde(default)]
    pub active: Option<bool>,
    /// Defaults to `false` when omitted.
    #[serde(default)]
    pub runenv: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAgentRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub runenv: Option<bool>,
    #[serde(default)]
    pub spec: Option<AgentSpec>,
}

pub(super) async fn list_agents(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<AgentListResponse>, ApiError> {
    let agents = match query.project_id {
        Some(pid) => state.agents.list_by_project(pid).await?,
        None => state.agents.list().await?,
    };
    Ok(Json(AgentListResponse {
        items: agents.into_iter().map(AgentResponse::from).collect(),
    }))
}

pub(super) async fn create_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<AgentResponse>), ApiError> {
    if state.projects.get(payload.project_id).await?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "project not found"));
    }

    let mut agent = Agent::new(payload.project_id, payload.name, payload.spec);
    if let Some(d) = payload.description {
        agent = agent.with_description(d);
    }
    if let Some(a) = payload.active {
        agent = agent.with_active(a);
    }
    if let Some(r) = payload.runenv {
        agent = agent.with_runenv(r);
    }
    state.agents.upsert(agent.clone()).await?;
    Ok((StatusCode::CREATED, Json(AgentResponse::from(agent))))
}

pub(super) async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentResponse>, ApiError> {
    let agent = state.agents.get(id).await?.ok_or(StateError::NotFound)?;
    Ok(Json(AgentResponse::from(agent)))
}

pub(super) async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateAgentRequest>,
) -> Result<Json<AgentResponse>, ApiError> {
    let mut agent = state.agents.get(id).await?.ok_or(StateError::NotFound)?;
    if let Some(n) = payload.name {
        agent = agent.with_name(n);
    }
    if let Some(d) = payload.description {
        agent = agent.with_description(d);
    }
    if let Some(a) = payload.active {
        agent = agent.with_active(a);
    }
    if let Some(r) = payload.runenv {
        agent = agent.with_runenv(r);
    }
    if let Some(s) = payload.spec {
        agent = agent.with_spec(s);
    }
    agent = agent.with_updated_at();
    state.agents.upsert(agent.clone()).await?;
    Ok(Json(AgentResponse::from(agent)))
}

pub(super) async fn delete_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.agents.remove(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
