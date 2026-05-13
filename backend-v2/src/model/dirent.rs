use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirentKind {
    File,
    Dir,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Dirent {
    pub path: String, // relative path from project uploads root
    pub kind: DirentKind,
    pub bytes: Option<u64>,                 // None for directories
    pub modified_at: Option<DateTime<Utc>>, // None if metadata unavailable
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UploadedFile {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FailedFile {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UploadResponse {
    pub project_id: Uuid,
    pub succeeded: Vec<UploadedFile>,
    pub failed: Vec<FailedFile>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub project_id: Uuid,
    pub entries: Vec<Dirent>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub prefix: Option<String>,
    pub recursive: Option<bool>,
}
