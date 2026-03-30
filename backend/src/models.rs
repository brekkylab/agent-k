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
    pub title: Option<String>,
    pub messages: Vec<SessionMessage>,
    pub speedwagon_ids: Vec<Uuid>,
    pub source_ids: Vec<Uuid>,
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
    pub title: Option<String>,
    #[serde(default)]
    pub speedwagon_ids: Vec<Uuid>,
    #[serde(default)]
    pub source_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    pub agent_id: Option<Uuid>,
    pub include_messages: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    pub provider_profile_id: Option<Uuid>,
    pub speedwagon_ids: Option<Vec<Uuid>>,
    pub source_ids: Option<Vec<Uuid>>,
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

// --- Source ---

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    LocalFile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: Uuid,
    pub name: String,
    pub source_type: SourceType,
    pub file_path: Option<String>,
    pub size: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SourceResponse {
    pub id: Uuid,
    pub name: String,
    pub source_type: SourceType,
    pub size: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&Source> for SourceResponse {
    fn from(s: &Source) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            source_type: s.source_type.clone(),
            size: s.size,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

// --- Speedwagon ---

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpeedwagonIndexStatus {
    NotIndexed,
    Indexing,
    Indexed,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Speedwagon {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub instruction: Option<String>,
    pub lm: Option<String>,
    pub source_ids: Vec<Uuid>,
    pub index_dir: Option<String>,
    pub corpus_dir: Option<String>,
    pub index_status: SpeedwagonIndexStatus,
    pub index_error: Option<String>,
    pub index_started_at: Option<DateTime<Utc>>,
    pub indexed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SpeedwagonResponse {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub instruction: Option<String>,
    pub lm: Option<String>,
    pub source_ids: Vec<Uuid>,
    pub index_dir: Option<String>,
    pub corpus_dir: Option<String>,
    pub index_status: SpeedwagonIndexStatus,
    pub index_error: Option<String>,
    pub index_started_at: Option<DateTime<Utc>>,
    pub indexed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&Speedwagon> for SpeedwagonResponse {
    fn from(s: &Speedwagon) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            description: s.description.clone(),
            instruction: s.instruction.clone(),
            lm: s.lm.clone(),
            source_ids: s.source_ids.clone(),
            index_dir: s.index_dir.clone(),
            corpus_dir: s.corpus_dir.clone(),
            index_status: s.index_status.clone(),
            index_error: s.index_error.clone(),
            index_started_at: s.index_started_at,
            indexed_at: s.indexed_at,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSpeedwagonRequest {
    pub name: String,
    pub description: String,
    pub instruction: Option<String>,
    pub lm: Option<String>,
    #[serde(default)]
    pub source_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSpeedwagonRequest {
    pub name: String,
    pub description: String,
    pub instruction: Option<String>,
    pub lm: Option<String>,
    #[serde(default)]
    pub source_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}
