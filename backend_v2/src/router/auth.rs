use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{Role, hash_password, validate_password, verify_password},
    state::{AppState, NewUser, User},
};

use super::error::{ApiError, err};

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

#[derive(Debug, Clone, Serialize, JsonSchema)]
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

impl From<User> for UserResponse {
    fn from(u: User) -> Self {
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

pub(super) async fn signup(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SignupRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    validate_password(&payload.password)?;

    let password_hash = hash_password(&payload.password)?;
    let id = Uuid::new_v4();

    let user = state
        .users
        .create(NewUser {
            id,
            username: payload.username,
            password_hash,
            role: Role::User,
            display_name: payload.display_name,
            is_active: true,
            preferred_language: "en".to_string(),
        })
        .await
        .map_err(|e| match e {
            crate::state::StateError::UniqueViolation(_) => {
                err(StatusCode::CONFLICT, "username already taken")
            }
            other => other.into(),
        })?;

    // Every user starts with one default workspace (its id mirrors the user's
    // id). The one-per-user policy lives here (not in a schema constraint), so
    // opening up multiple workspaces later is just a matter of exposing a
    // create endpoint.
    state.workspaces.create_default(&user).await?;

    tracing::info!(%id, username = %user.username, "user signed up");

    Ok((StatusCode::CREATED, Json(UserResponse::from(user))))
}

pub(super) async fn login(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let user = state
        .users
        .get_by_username(&payload.username)
        .await?
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "invalid username or password"))?;

    if !user.is_active {
        return Err(err(StatusCode::FORBIDDEN, "account is deactivated"));
    }

    if !verify_password(&payload.password, &user.password_hash)? {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid username or password"));
    }

    // Heal a missing default workspace (e.g. a signup that failed after the
    // user row was created): provision it lazily on login.
    if state.workspaces.get(user.id).await?.is_none() {
        state.workspaces.create_default(&user).await?;
    }

    let access_token = state
        .jwt
        .encode(user.id, user.username.clone(), user.role.clone())?;

    tracing::info!(id = %user.id, username = %user.username, "user logged in");

    Ok(Json(LoginResponse {
        token_type: "Bearer".to_string(),
        expires_in: state.jwt.expiry_secs,
        user: UserResponse::from(user),
        access_token,
    }))
}
