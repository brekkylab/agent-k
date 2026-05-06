use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {}

/// API response for GET /sessions (list) and POST /sessions (create) -- no messages
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SessionResponse {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
