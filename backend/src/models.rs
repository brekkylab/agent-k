use ailoy::{AgentProvider as RuntimeAgentProvider, AgentSpec as RuntimeAgentSpec};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::agent::spec::{AgentProvider, AgentSpec};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub spec: RuntimeAgentSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AgentResponse {
    pub id: Uuid,
    pub spec: AgentSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub id: Uuid,
    pub name: String,
    pub provider: RuntimeAgentProvider,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ProviderProfileResponse {
    pub id: Uuid,
    pub name: String,
    pub provider: AgentProvider,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionMessage {
    pub role: MessageRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Session {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub provider_profile_id: Uuid,
    pub messages: Vec<SessionMessage>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentRequest {
    pub spec: AgentSpec,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAgentRequest {
    pub spec: AgentSpec,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProviderProfileRequest {
    pub name: String,
    pub provider: AgentProvider,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProviderProfileRequest {
    pub name: String,
    pub provider: AgentProvider,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {
    pub agent_id: Uuid,
    pub provider_profile_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    pub agent_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddSessionMessageRequest {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AddSessionMessageResponse {
    pub assistant_message: Option<SessionMessage>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}
