use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    auth::role::Role,
    router::error::err,
    state::AppState,
};

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
        None => return err(StatusCode::UNAUTHORIZED, "missing bearer token").into_response(),
    };

    let claims = match state.jwt.decode(&token) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let user = match state.users.get(claims.sub).await {
        Ok(Some(u)) => u,
        Ok(None) => return err(StatusCode::UNAUTHORIZED, "user not found").into_response(),
        Err(e) => {
            tracing::error!("auth user lookup failed: {e}");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    };

    if !user.is_active {
        return err(StatusCode::FORBIDDEN, "account is deactivated").into_response();
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
        None => err(StatusCode::UNAUTHORIZED, "authentication required").into_response(),
        Some(r) if r != Role::Admin => {
            err(StatusCode::FORBIDDEN, "admin access required").into_response()
        }
        _ => next.run(request).await,
    }
}
