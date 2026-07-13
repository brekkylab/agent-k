use std::sync::Arc;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{auth::role::Role, error::AppError, state::AppState};

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub role: Role,
}

pub async fn auth_required(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let token = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);

    let token = match token {
        Some(t) => t,
        None => return AppError::unauthorized("missing bearer token").into_response(),
    };

    let claims = match state.jwt.decode(&token) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let user = match state
        .repository
        .get_user_by_id(claims.sub)
        .await
        .map_err(|e| AppError::internal(e.to_string()))
    {
        Ok(Some(u)) => u,
        Ok(None) => return AppError::unauthorized("user not found").into_response(),
        Err(e) => return e.into_response(),
    };

    if !user.is_active {
        return AppError::forbidden("account is deactivated").into_response();
    }

    request.extensions_mut().insert(AuthUser {
        id: user.id,
        username: user.username,
        role: user.role,
    });

    next.run(request).await
}

pub async fn admin_required(request: Request, next: Next) -> Response {
    let role = request
        .extensions()
        .get::<AuthUser>()
        .map(|u| u.role.clone());

    match role {
        None => AppError::unauthorized("authentication required").into_response(),
        Some(r) if r != Role::Admin => AppError::forbidden("admin access required").into_response(),
        _ => next.run(request).await,
    }
}
