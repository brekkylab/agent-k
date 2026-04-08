use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- Enums ---

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpeedwagonIndexStatus {
    NotIndexed,
    Indexing,
    Indexed,
    Error,
}

// --- Domain Models ---

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Speedwagon {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub instruction: Option<String>,
    pub lm: Option<String>,
    pub provider_profile_id: Option<Uuid>,
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

// --- Response DTOs ---

#[derive(Debug, Serialize, JsonSchema)]
pub struct SpeedwagonResponse {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub instruction: Option<String>,
    pub lm: Option<String>,
    pub provider_profile_id: Option<Uuid>,
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
            provider_profile_id: s.provider_profile_id,
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

// --- Request DTOs ---

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSpeedwagonRequest {
    pub name: String,
    pub description: String,
    pub instruction: Option<String>,
    pub lm: Option<String>,
    #[serde(default)]
    pub provider_profile_id: Option<Uuid>,
    #[serde(default)]
    pub source_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSpeedwagonRequest {
    pub name: String,
    pub description: String,
    pub instruction: Option<String>,
    pub lm: Option<String>,
    #[serde(default)]
    pub provider_profile_id: Option<Uuid>,
    #[serde(default)]
    pub source_ids: Vec<Uuid>,
}
