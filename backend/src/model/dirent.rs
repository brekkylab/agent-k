use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirentKind {
    File,
    Dir,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Dirent {
    pub path: String, // global path relative to data_root (e.g. projects/{pid}/shared/foo.txt)
    pub kind: DirentKind,
    pub bytes: Option<u64>,                 // None for directories
    pub modified_at: Option<DateTime<Utc>>, // None if metadata unavailable
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FailedFile {
    pub path: String,
    pub error: String,
}

/// Unified batch operation result: upload / move / copy all use this shape.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DirentBatchResult {
    pub succeeded: Vec<Dirent>,
    pub failed: Vec<FailedFile>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub entries: Vec<Dirent>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub prefix: Option<String>,
    pub recursive: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DirentScopeQuery {
    pub path: String,
}

/// Tagged union for collection-level PATCH /dirents operations.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirentBatchOp {
    /// Move (and optionally rename) one or more items to a destination folder.
    /// `new_name` is allowed only when `sources.len() == 1` (the rename case).
    Move {
        sources: Vec<String>,
        destination: String,
        new_name: Option<String>,
    },
    /// Copy one or more items into a destination folder. Name collisions get
    /// a " copy" suffix automatically (Finder-style).
    Copy {
        sources: Vec<String>,
        destination: String,
    },
}
