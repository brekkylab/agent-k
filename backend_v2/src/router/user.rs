use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{AuthUser, Role, hash_password, validate_password, verify_password},
    state::{AppState, NewUser, StateError, UpdateUser},
};

use super::{
    auth::UserResponse,
    error::{ApiError, err},
};

const DEFAULT_LANGUAGE: &str = "en";

fn validate_language(lang: &str) -> Result<(), ApiError> {
    if matches!(lang, "en" | "ko") {
        Ok(())
    } else {
        Err(err(
            StatusCode::BAD_REQUEST,
            "preferred_language must be 'en' or 'ko'",
        ))
    }
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

pub(super) async fn get_me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state
        .users
        .get(auth.id)
        .await?
        .ok_or(StateError::NotFound)?;
    Ok(Json(UserResponse::from(user)))
}

pub(super) async fn update_me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Json(payload): Json<UpdateMeRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let new_password_hash = if let Some(ref new_password) = payload.password {
        validate_password(new_password)?;

        let current_password = payload.current_password.as_deref().ok_or_else(|| {
            err(
                StatusCode::BAD_REQUEST,
                "current_password is required to change password",
            )
        })?;

        let user = state
            .users
            .get(auth.id)
            .await?
            .ok_or(StateError::NotFound)?;

        if !verify_password(current_password, &user.password_hash)? {
            return Err(err(StatusCode::UNAUTHORIZED, "current password is incorrect"));
        }

        Some(hash_password(new_password)?)
    } else {
        None
    };

    if let Some(ref lang) = payload.preferred_language {
        validate_language(lang)?;
    }

    let updated = state
        .users
        .update(
            auth.id,
            UpdateUser {
                display_name: payload.display_name,
                password_hash: new_password_hash,
                role: None,
                is_active: None,
                preferred_language: payload.preferred_language,
            },
        )
        .await?
        .ok_or(StateError::NotFound)?;

    Ok(Json(UserResponse::from(updated)))
}

pub(super) async fn list_users(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Query(q): Query<UserListQuery>,
) -> Result<Json<UserListResponse>, ApiError> {
    let page = q.page.unwrap_or(1);
    let size = q.size.unwrap_or(20);

    let (users, total) = state.users.list(page, size).await?;

    Ok(Json(UserListResponse {
        items: users.into_iter().map(UserResponse::from).collect(),
        total,
    }))
}

pub(super) async fn create_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Json(payload): Json<AdminCreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    validate_password(&payload.password)?;

    let password_hash = hash_password(&payload.password)?;
    let id = Uuid::new_v4();
    let role = payload.role.unwrap_or(Role::User);
    let is_active = payload.is_active.unwrap_or(true);
    let preferred_language = payload
        .preferred_language
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
    validate_language(&preferred_language)?;

    let user = state
        .users
        .create(NewUser {
            id,
            username: payload.username,
            password_hash,
            role,
            display_name: payload.display_name,
            is_active,
            preferred_language,
        })
        .await
        .map_err(|e| match e {
            StateError::UniqueViolation(_) => {
                err(StatusCode::CONFLICT, "username already taken")
            }
            other => other.into(),
        })?;

    tracing::info!(%id, username = %user.username, "admin created user");

    Ok((StatusCode::CREATED, Json(UserResponse::from(user))))
}

pub(super) async fn get_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = state
        .users
        .get(id)
        .await?
        .ok_or(StateError::NotFound)?;
    Ok(Json(UserResponse::from(user)))
}

pub(super) async fn update_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(payload): Json<AdminUpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let removes_admin_access = matches!(&payload.role, Some(r) if *r != Role::Admin)
        || matches!(payload.is_active, Some(false));

    if auth.id == id {
        if removes_admin_access {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "cannot remove your own admin access",
            ));
        }
    } else if removes_admin_access {
        let target = state
            .users
            .get(id)
            .await?
            .ok_or(StateError::NotFound)?;
        if target.role == Role::Admin && target.is_active {
            let count = state.users.count_admins().await?;
            if count <= 1 {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "cannot remove the last active admin",
                ));
            }
        }
    }

    let new_password_hash = payload
        .password
        .as_deref()
        .map(|p| {
            validate_password(p)?;
            hash_password(p)
        })
        .transpose()?;

    if let Some(ref lang) = payload.preferred_language {
        validate_language(lang)?;
    }

    let updated = state
        .users
        .update(
            id,
            UpdateUser {
                display_name: payload.display_name,
                password_hash: new_password_hash,
                role: payload.role,
                is_active: payload.is_active,
                preferred_language: payload.preferred_language,
            },
        )
        .await?
        .ok_or(StateError::NotFound)?;

    Ok(Json(UserResponse::from(updated)))
}

pub(super) async fn delete_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if auth.id == id {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "cannot delete your own account",
        ));
    }

    let deleted = state.users.delete(id).await?;
    if !deleted {
        return Err(StateError::NotFound.into());
    }

    tracing::info!(target_user_id = %id, by = %auth.id, "admin deleted user");

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::validate_language;

    #[test]
    fn validate_language_accepts_en_and_ko() {
        assert!(validate_language("en").is_ok());
        assert!(validate_language("ko").is_ok());
    }

    #[test]
    fn validate_language_rejects_unsupported_codes() {
        assert!(validate_language("ja").is_err());
        assert!(validate_language("zh").is_err());
        assert!(validate_language("").is_err());
    }

    #[test]
    fn validate_language_is_case_sensitive() {
        assert!(validate_language("EN").is_err());
        assert!(validate_language("Ko").is_err());
    }
}
