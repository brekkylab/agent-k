use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::{
    auth::{AuthUser, Role, hash_password, validate_password, verify_password},
    error::{ApiResult, AppError},
    app_tools::AgentPolicy,
    model::{
        AdminCreateUserRequest, AdminUpdateUserRequest, MeResponse, UpdateMeRequest, UserListQuery,
        UserListResponse, UserResponse,
    },
    repository::{NewUser, RepositoryError, UpdateUser},
    state::AppState,
};

const DEFAULT_LANGUAGE: &str = "en";

fn validate_language(lang: &str) -> ApiResult<()> {
    if matches!(lang, "en" | "ko") {
        Ok(())
    } else {
        Err(AppError::bad_request(
            "preferred_language must be 'en' or 'ko'",
        ))
    }
}

pub async fn get_me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
) -> ApiResult<Json<MeResponse>> {
    let user = state
        .repository
        .get_user_by_id(auth.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(MeResponse {
        user: UserResponse::from(user),
        agent_capabilities: AgentPolicy::for_user(auth.id).granted_names(),
    }))
}

pub async fn update_me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Json(payload): Json<UpdateMeRequest>,
) -> ApiResult<Json<MeResponse>> {
    let new_password_hash = if let Some(ref new_password) = payload.password {
        validate_password(new_password)?;

        let current_password = payload.current_password.as_deref().ok_or_else(|| {
            AppError::bad_request("current_password is required to change password")
        })?;

        let user = state
            .repository
            .get_user_by_id(auth.id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
            .ok_or_else(|| AppError::not_found("user not found"))?;

        if !verify_password(current_password, &user.password_hash)? {
            return Err(AppError::unauthorized("current password is incorrect"));
        }

        Some(hash_password(new_password)?)
    } else {
        None
    };

    if let Some(ref lang) = payload.preferred_language {
        validate_language(lang)?;
    }

    let updated = state
        .repository
        .update_user(
            auth.id,
            UpdateUser {
                display_name: payload.display_name,
                password_hash: new_password_hash,
                role: None,
                is_active: None,
                preferred_language: payload.preferred_language,
            },
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(MeResponse {
        user: UserResponse::from(updated),
        agent_capabilities: AgentPolicy::for_user(auth.id).granted_names(),
    }))
}

pub async fn list_users(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Query(q): Query<UserListQuery>,
) -> ApiResult<Json<UserListResponse>> {
    let page = q.page.unwrap_or(1);
    let size = q.size.unwrap_or(20);

    let (users, total) = state
        .repository
        .list_users(page, size)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(UserListResponse {
        items: users.into_iter().map(UserResponse::from).collect(),
        total,
    }))
}

pub async fn create_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Json(payload): Json<AdminCreateUserRequest>,
) -> ApiResult<(StatusCode, Json<UserResponse>)> {
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
        .repository
        .create_user(NewUser {
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
            RepositoryError::UniqueViolation(_) => AppError::conflict("username already taken"),
            other => AppError::internal(other.to_string()),
        })?;

    tracing::info!(%id, username = %user.username, "admin created user");

    Ok((StatusCode::CREATED, Json(UserResponse::from(user))))
}

pub async fn get_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(_auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<UserResponse>> {
    let user = state
        .repository
        .get_user_by_id(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(UserResponse::from(user)))
}

pub async fn update_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(payload): Json<AdminUpdateUserRequest>,
) -> ApiResult<Json<UserResponse>> {
    let removes_admin_access = matches!(&payload.role, Some(r) if *r != Role::Admin)
        || matches!(payload.is_active, Some(false));

    if auth.id == id {
        if removes_admin_access {
            return Err(AppError::bad_request("cannot remove your own admin access"));
        }
    } else if removes_admin_access {
        // Prevent demoting or deactivating the last active admin.
        let target = state
            .repository
            .get_user_by_id(id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
            .ok_or_else(|| AppError::not_found("user not found"))?;
        if target.role == Role::Admin && target.is_active {
            let count = state
                .repository
                .count_admins()
                .await
                .map_err(|e| AppError::internal(e.to_string()))?;
            if count <= 1 {
                return Err(AppError::bad_request("cannot remove the last active admin"));
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
        .repository
        .update_user(
            id,
            UpdateUser {
                display_name: payload.display_name,
                password_hash: new_password_hash,
                role: payload.role,
                is_active: payload.is_active,
                preferred_language: payload.preferred_language,
            },
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(UserResponse::from(updated)))
}

pub async fn delete_user_admin(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if auth.id == id {
        return Err(AppError::bad_request("cannot delete your own account"));
    }

    let deleted = state
        .repository
        .delete_user(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    if !deleted {
        return Err(AppError::not_found("user not found"));
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
        // The frontend always sends lowercase codes; reject uppercase so a typo
        // doesn't silently succeed in the DB.
        assert!(validate_language("EN").is_err());
        assert!(validate_language("Ko").is_err());
    }
}
