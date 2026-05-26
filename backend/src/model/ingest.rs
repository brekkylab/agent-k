use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

use crate::repository::DbIngestJob;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IngestStatus {
    Queued,
    Running,
    Done,
    Failed,
}

impl IngestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            IngestStatus::Queued => "queued",
            IngestStatus::Running => "running",
            IngestStatus::Done => "done",
            IngestStatus::Failed => "failed",
        }
    }

    pub fn from_db(s: &str) -> Result<Self, String> {
        Ok(match s {
            "queued" => IngestStatus::Queued,
            "running" => IngestStatus::Running,
            "done" => IngestStatus::Done,
            "failed" => IngestStatus::Failed,
            other => return Err(format!("invalid ingest_jobs.status: {other}")),
        })
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct IngestJobResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub source_path: String,
    pub status: IngestStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub document_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DbIngestJob> for IngestJobResponse {
    fn from(j: DbIngestJob) -> Self {
        Self {
            id: j.id,
            project_id: j.project_id,
            source_path: j.source_path,
            status: j.status,
            attempts: j.attempts,
            last_error: j.last_error,
            document_id: j.document_id,
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}

/// True for file extensions that the auto-ingest pipeline recognizes.
/// Mirrors `handlers::document::parse_filetype`.
pub fn is_ingestable_filename(filename: &str) -> bool {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "pdf" | "md" | "markdown" | "txt")
}
