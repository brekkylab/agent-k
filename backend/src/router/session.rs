use std::sync::Arc;

use agent_k::agents::{get_coworker_agent_spec, get_deep_research_agent_spec};
use ailoy::agent::AgentSpec;
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
    state::{AppState, Session},
};

use super::{
    error::{ApiError, err},
    workspace::{require_owned_agent, require_owned_session, require_owned_workspace},
};

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
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
            workspace_id: s.workspace_id,
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

/// Selects which agent-k preset builds the [`AgentSpec`] when creating a
/// session. Variants correspond 1:1 to the `get_*_agent_spec` family in
/// [`agent_k::agents`]; [`build_spec`] is the dispatch.
///
/// Wire values are kebab-case, matching the surface names `GET /models`
/// advertises ([`crate::model::AgentType`]); `deep_research` stays accepted
/// as a legacy alias.
// TODO: add `Speedwagon` variant once the knowledge-base store wiring is ready.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    Coworker,
    #[serde(alias = "deep_research")]
    DeepResearch,
}

impl AgentType {
    /// The matching catalog surface, whose recommendation chain drives model
    /// resolution in [`build_spec`].
    fn catalog(self) -> crate::model::AgentType {
        match self {
            AgentType::Coworker => crate::model::AgentType::Coworker,
            AgentType::DeepResearch => crate::model::AgentType::DeepResearch,
        }
    }
}

/// An absent or unavailable `model` falls back to the agent-type's catalog
/// chain (first configured provider) rather than a fixed default.
fn build_spec(agent_type: AgentType, model: Option<&str>) -> AgentSpec {
    let model = crate::model::resolve_model(Some(agent_type.catalog().as_str()), model);
    match agent_type {
        AgentType::Coworker => get_coworker_agent_spec(SESSION_AGENT_NAME, &model, true),
        AgentType::DeepResearch => get_deep_research_agent_spec(SESSION_AGENT_NAME, &model),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {
    pub workspace_id: Uuid,
    pub title: Option<String>,
    /// Create the session from a stored, workspace-scoped agent. When set, the
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
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let sessions = state.sessions.list_by_workspace(auth.id).await?;
    Ok(Json(SessionListResponse {
        items: sessions.into_iter().map(SessionResponse::from).collect(),
    }))
}

pub(super) async fn create_session(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError> {
    require_owned_workspace(&state, &auth, payload.workspace_id).await?;

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
            // require_owned_agent 404s for missing/foreign agents alike; owning
            // the agent's workspace also implies it is this one (both == caller id).
            let agent = require_owned_agent(&state, &auth, agent_id).await?;
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

    let mut session = Session::new(payload.workspace_id, spec);
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
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionResponse>, ApiError> {
    let session = require_owned_session(&state, &auth, id).await?;
    Ok(Json(SessionResponse::from(session)))
}

pub(super) async fn delete_session(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_owned_session(&state, &auth, id).await?;
    state.delete_session(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<AgentType, serde_json::Error> {
        serde_json::from_str(&format!("\"{s}\""))
    }

    #[test]
    fn agent_type_accepts_catalog_wire_values() {
        // Every surface name `GET /models` advertises must post back into a
        // session unchanged.
        for catalog in crate::model::AgentType::ALL {
            let parsed = parse(catalog.as_str())
                .unwrap_or_else(|e| panic!("{} must parse: {e}", catalog.as_str()));
            assert_eq!(parsed.catalog(), catalog);
        }
    }

    #[test]
    fn agent_type_accepts_legacy_snake_case_alias() {
        assert!(matches!(parse("deep_research"), Ok(AgentType::DeepResearch)));
        assert!(parse("deep research").is_err());
        assert!(parse("speedwagon").is_err());
    }
}
