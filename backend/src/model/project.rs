use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::DbProject;

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectResponse {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_id: Uuid,
    /// Per-agent_type recommendation-chain overrides (only the agent types this
    /// project customized; missing ones use the built-in default chain).
    pub recommended_chains: BTreeMap<String, Vec<String>>,
    /// Knowledge-corpus PDF engine: "kreuzberg" (default) | "docling".
    pub pdf_engine: String,
    /// Project-level ceiling on agent capabilities (capability names).
    /// `null` = no limit (all capabilities allowed). Owner-editable.
    pub agent_capability_ceiling: Option<Vec<String>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DbProject> for ProjectResponse {
    fn from(p: DbProject) -> Self {
        Self {
            id: p.id,
            slug: p.slug,
            name: p.name,
            description: p.description,
            owner_id: p.owner_id,
            recommended_chains: p
                .recommended_chains
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default(),
            pdf_engine: p
                .pdf_engine
                .unwrap_or_else(|| agent_k::knowledge_base::PdfEngine::default().as_str().to_string()),
            agent_capability_ceiling: p
                .agent_capability_ceiling
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
    /// Optional slug override. When omitted, the server generates one from `name`.
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Replace the project's recommendation-chain overrides (JSON object keyed
    /// by agent_type). Omit to leave unchanged; send `{}` to reset all to default.
    #[serde(default)]
    pub recommended_chains: Option<BTreeMap<String, Vec<String>>>,
    /// Knowledge-corpus PDF engine: "kreuzberg" | "docling". Omit to leave unchanged.
    #[serde(default)]
    pub pdf_engine: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectMemberResponse {
    pub user_id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub added_at: DateTime<Utc>,
    /// This member's per-project agent capability grant (capability names).
    /// `null` = unset (inherits the project ceiling). Owner has no row → `null`.
    pub agent_capabilities: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddMemberRequest {
    pub username: String,
}

/// Set an agent capability set (project ceiling or a member's grant).
/// `capabilities: null` clears it (ceiling → no limit; member → inherit ceiling).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAgentCapabilitiesRequest {
    pub capabilities: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectListResponse {
    pub items: Vec<ProjectResponse>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectMemberListResponse {
    pub items: Vec<ProjectMemberResponse>,
}
