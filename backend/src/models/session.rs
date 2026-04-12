use std::collections::HashMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- Enums ---

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

// --- Domain Models ---

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SessionMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SessionToolCall {
    pub id: String,
    pub message_id: String,
    pub tool_name: String,
    pub tool_args: Option<serde_json::Value>,
    pub tool_result: Option<serde_json::Value>,
    pub duration_ms: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
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

// --- Response DTOs ---

/// API response for GET /sessions (list) and POST /sessions (create) -- no messages
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SessionResponse {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub provider_profile_id: Uuid,
    pub title: Option<String>,
    pub speedwagon_ids: Vec<Uuid>,
    pub source_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&Session> for SessionResponse {
    fn from(s: &Session) -> Self {
        Self {
            id: s.id,
            agent_id: s.agent_id,
            provider_profile_id: s.provider_profile_id,
            title: s.title.clone(),
            speedwagon_ids: s.speedwagon_ids.clone(),
            source_ids: s.source_ids.clone(),
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// API response message -- includes tool calls (only populated in GET /sessions/{id})
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SessionMessageResponse {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<SessionToolCall>,
    pub created_at: DateTime<Utc>,
}

impl From<SessionMessage> for SessionMessageResponse {
    fn from(m: SessionMessage) -> Self {
        Self {
            id: m.id,
            role: m.role,
            content: m.content,
            tool_calls: vec![],
            created_at: m.created_at,
        }
    }
}

/// API response for GET /sessions/{id} -- includes messages with tool calls
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SessionDetailResponse {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub provider_profile_id: Uuid,
    pub title: Option<String>,
    pub messages: Vec<SessionMessageResponse>,
    pub speedwagon_ids: Vec<Uuid>,
    pub source_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<(Session, Vec<SessionToolCall>)> for SessionDetailResponse {
    fn from((session, tool_calls): (Session, Vec<SessionToolCall>)) -> Self {
        let mut tc_map: HashMap<String, Vec<SessionToolCall>> = HashMap::new();
        for tc in tool_calls {
            tc_map.entry(tc.message_id.clone()).or_default().push(tc);
        }
        let messages = session
            .messages
            .into_iter()
            .map(|m| {
                let msg_id = m.id.clone();
                let mut resp = SessionMessageResponse::from(m);
                resp.tool_calls = tc_map.remove(&msg_id).unwrap_or_default();
                resp
            })
            .collect();
        Self {
            id: session.id,
            agent_id: session.agent_id,
            provider_profile_id: session.provider_profile_id,
            title: session.title,
            messages,
            speedwagon_ids: session.speedwagon_ids,
            source_ids: session.source_ids,
            created_at: session.created_at,
            updated_at: session.updated_at,
        }
    }
}

// --- Request DTOs ---

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSessionsQuery {
    pub agent_id: Option<Uuid>,
    pub include_messages: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    pub agent_id: Option<Uuid>,
    pub provider_profile_id: Option<Uuid>,
    pub speedwagon_ids: Option<Vec<Uuid>>,
    pub source_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddSessionMessageRequest {
    pub content: String,
}
