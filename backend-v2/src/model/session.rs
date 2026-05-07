use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::DbSession;

pub use crate::repository::ShareMode;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {}

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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DbSession> for SessionResponse {
    fn from(s: DbSession) -> Self {
        Self {
            id: s.id,
            project_id: s.project_id,
            creator_id: s.creator_id,
            share_mode: s.share_mode,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}
