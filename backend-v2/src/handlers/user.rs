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
    model::{
        AdminCreateUserRequest, AdminUpdateUserRequest, UpdateMeRequest, UserListQuery,
        UserListResponse, UserResponse,
    },
    repository::{NewUser, RepositoryError, UpdateUser},
    state::AppState,
};

pub async fn get_me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
) -> ApiResult<Json<UserResponse>> {
    let user = state
        .repository
        .get_user_by_id(auth.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(UserResponse::from(user)))
}

pub async fn update_me(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Json(payload): Json<UpdateMeRequest>,
) -> ApiResult<Json<UserResponse>> {
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

    let updated = state
        .repository
        .update_user(
            auth.id,
            UpdateUser {
                display_name: payload.display_name,
                password_hash: new_password_hash,
                role: None,
                is_active: None,
            },
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    Ok(Json(UserResponse::from(updated)))
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

    let user = state
        .repository
        .create_user(NewUser {
            id,
            username: payload.username,
            password_hash,
            role,
            display_name: payload.display_name,
            is_active,
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
    Extension(_auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(payload): Json<AdminUpdateUserRequest>,
) -> ApiResult<Json<UserResponse>> {
    let new_password_hash = payload
        .password
        .as_deref()
        .map(|p| {
            validate_password(p)?;
            hash_password(p)
        })
        .transpose()?;

    let updated = state
        .repository
        .update_user(
            id,
            UpdateUser {
                display_name: payload.display_name,
                password_hash: new_password_hash,
                role: payload.role,
                is_active: payload.is_active,
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
