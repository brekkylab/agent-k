use ailoy::message::{Message, MessageOutput};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::DbSession;
pub use crate::repository::{SessionOrigin, ShareMode};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {
    pub project_id: Uuid,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSessionRequest {
    pub share_mode: ShareMode,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SessionResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub creator_id: Uuid,
    pub share_mode: ShareMode,
    pub origin: SessionOrigin,
    pub title: Option<String>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub last_message_snippet: Option<String>,
    pub unread_count: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SessionResponse {
    pub fn from_db(s: DbSession, unread_count: u64) -> Self {
        Self {
            id: s.id,
            project_id: s.project_id,
            creator_id: s.creator_id,
            share_mode: s.share_mode,
            origin: s.origin,
            title: s.title,
            last_message_at: s.last_message_at,
            last_message_snippet: s.last_message_snippet,
            unread_count,
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
pub struct SendMessageRequest {
    pub content: String,
}

pub type SendMessageResponse = Vec<MessageOutput>;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageSender {
    User { user_id: Uuid },
    Agent { name: String },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SessionMessageResponse {
    pub message: Message,
    pub sender: MessageSender,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionMessageListResponse {
    pub items: Vec<SessionMessageResponse>,
}
