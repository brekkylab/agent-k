use ailoy::message::Message;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::{DbSession, UnreadInfo};
pub use crate::repository::{SessionOrigin, ShareMode};

/// Message channel: 'chat' goes to the agent, 'team' is user-to-user only.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Chat,
    Team,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {
    /// Project UUID, active slug, or retired slug — backend resolves all three.
    pub project_ref: String,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
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
    pub agent_type: Option<String>,
    pub model: Option<String>,
    /// Whether the pinned `model`'s provider is currently configured. `true`
    /// when there is no pin (recommended). When `false`, agent-build falls back
    /// to an available model, so the UI can flag that `model` isn't what runs.
    pub model_available: bool,
    pub unread_count: u64,
    /// True when an unread message mentions the requesting user.
    pub unread_mention: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SessionResponse {
    pub fn from_db(s: DbSession, unread: UnreadInfo) -> Self {
        Self {
            id: s.id,
            project_id: s.project_id,
            creator_id: s.creator_id,
            share_mode: s.share_mode,
            origin: s.origin,
            title: s.title,
            last_message_at: s.last_message_at,
            last_message_snippet: s.last_message_snippet,
            model_available: s
                .model
                .as_deref()
                .filter(|m| !m.is_empty())
                .map_or(true, crate::model::provider_available),
            agent_type: s.agent_type,
            model: s.model,
            unread_count: unread.count,
            unread_mention: unread.mentioned,
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
    pub attachments: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendTeamMessageRequest {
    pub content: String,
    pub attachments: Option<Vec<String>>,
    /// Project-member user ids mentioned in the message.
    pub mentions: Option<Vec<Uuid>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunAck {
    pub status: &'static str,
    pub run_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunActiveResponse {
    pub active: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageSender {
    User { user_id: Uuid },
    Agent { name: String },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SessionMessageResponse {
    /// Session-global insertion order — stable client identity across windows.
    pub seq: i64,
    pub message: Message,
    pub sender: MessageSender,
    pub created_at: DateTime<Utc>,
    pub attachments: Vec<String>,
    /// Scope-relative paths of artifacts created during this message turn.
    pub artifacts: Vec<String>,
    /// Citations parsed from a Speedwagon answer and checked against the corpus.
    /// Empty for non-Speedwagon messages or answers without citations.
    #[serde(default)]
    pub citations: Vec<crate::handlers::knowledge::CitationCheck>,
    /// Named message_kind (not `kind`) — `sender` already uses a `kind` tag.
    pub message_kind: MessageKind,
    /// User ids mentioned by a team message.
    pub mentions: Vec<Uuid>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionMessageListResponse {
    pub items: Vec<SessionMessageResponse>,
}
