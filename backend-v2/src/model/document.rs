use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use speedwagon::Document;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DocumentResponse {
    pub id: String,
    pub title: String,
    pub len: usize,
}

impl From<&Document> for DocumentResponse {
    fn from(doc: &Document) -> Self {
        Self {
            id: doc.id.clone(),
            title: doc.title.clone(),
            len: doc.len,
        }
    }
}

impl From<Document> for DocumentResponse {
    fn from(doc: Document) -> Self {
        Self {
            id: doc.id,
            title: doc.title,
            len: doc.len,
        }
    }
}

// --- Response DTOs ---

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FailedItem {
    pub name: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BatchIngestResponse {
    pub succeeded: Vec<DocumentResponse>,
    pub failed: Vec<FailedItem>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct BatchPurgeResponse {
    pub purged: Vec<String>,
    pub failed: Vec<FailedItem>,
}

// --- Request DTOs ---

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BulkPurgeRequest {
    pub ids: Vec<Uuid>,
}
