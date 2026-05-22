use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repository::DbAutomation;

#[derive(Debug, Serialize, JsonSchema)]
pub struct AutomationResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub prompts: Vec<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DbAutomation> for AutomationResponse {
    fn from(a: DbAutomation) -> Self {
        Self {
            id: a.id,
            project_id: a.project_id,
            name: a.name,
            description: a.description,
            prompts: a.prompts,
            created_by: a.created_by,
            created_at: a.created_at,
            updated_at: a.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAutomationRequest {
    pub project_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub prompts: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAutomationRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub prompts: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AutomationListResponse {
    pub items: Vec<AutomationResponse>,
}
