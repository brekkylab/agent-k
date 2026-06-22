use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{authn::Role, repository::DbUser};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignupRequest {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LoginResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub user: UserResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserResponse {
    pub id: Uuid,
    pub username: String,
    pub role: Role,
    pub display_name: Option<String>,
    pub is_active: bool,
    pub preferred_language: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DbUser> for UserResponse {
    fn from(u: DbUser) -> Self {
        Self {
            id: u.id,
            username: u.username,
            role: u.role,
            display_name: u.display_name,
            is_active: u.is_active,
            preferred_language: u.preferred_language,
            created_at: u.created_at,
            updated_at: u.updated_at,
        }
    }
}

/// `/me` payload: the user's profile plus the capabilities granted to their
/// agents (read-only in the UI for now). Flattens [`UserResponse`] so existing
/// clients that read user fields keep working.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MeResponse {
    #[serde(flatten)]
    pub user: UserResponse,
    /// Stable capability names (e.g. `automation.read`) the user's agents may use.
    pub agent_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateMeRequest {
    pub display_name: Option<String>,
    pub password: Option<String>,
    pub current_password: Option<String>,
    pub preferred_language: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminCreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: Option<Role>,
    pub display_name: Option<String>,
    pub is_active: Option<bool>,
    pub preferred_language: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminUpdateUserRequest {
    pub password: Option<String>,
    pub role: Option<Role>,
    pub display_name: Option<String>,
    pub is_active: Option<bool>,
    pub preferred_language: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UserListResponse {
    pub items: Vec<UserResponse>,
    pub total: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UserListQuery {
    pub page: Option<u32>,
    pub size: Option<u32>,
}
