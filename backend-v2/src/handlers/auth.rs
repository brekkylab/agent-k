use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use crate::{
    auth::{Role, hash_password, validate_password, verify_password},
    error::{ApiResult, AppError},
    model::{LoginRequest, LoginResponse, SignupRequest, UserResponse},
    repository::{NewUser, RepositoryError},
    state::AppState,
};

pub async fn signup(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SignupRequest>,
) -> ApiResult<(StatusCode, Json<UserResponse>)> {
    validate_password(&payload.password)?;

    let password_hash = hash_password(&payload.password)?;
    let id = Uuid::new_v4();

    let user = state
        .repository
        .create_user(NewUser {
            id,
            username: payload.username,
            password_hash,
            role: Role::User,
            display_name: payload.display_name,
            is_active: true,
        })
        .await
        .map_err(|e| match e {
            RepositoryError::UniqueViolation(_) => AppError::conflict("username already taken"),
            other => AppError::internal(other.to_string()),
        })?;

    tracing::info!(%id, username = %user.username, "user signed up");

    Ok((StatusCode::CREATED, Json(UserResponse::from(user))))
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let user = state
        .repository
        .get_user_by_username(&payload.username)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::unauthorized("invalid username or password"))?;

    if !user.is_active {
        return Err(AppError::forbidden("account is deactivated"));
    }

    if !verify_password(&payload.password, &user.password_hash)? {
        return Err(AppError::unauthorized("invalid username or password"));
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
