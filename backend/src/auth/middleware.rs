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
    router::error::{ApiError, err},
    state::AppState,
};

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub role: Role,
}

/// Decode `token`, load the subject, and require an active account, returning
/// the authenticated principal. The single authentication gate shared by the
/// `auth_required` middleware and the token-in-query endpoints (WebDAV and the
/// message WebSocket) that bypass the middleware.
pub async fn authenticate(state: &AppState, token: &str) -> Result<AuthUser, ApiError> {
    let claims = state.jwt.decode(token)?;

    let user = state
        .users
        .get(claims.sub)
        .await
        .map_err(|e| {
            tracing::error!("auth user lookup failed: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        })?
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "user not found"))?;

    if !user.is_active {
        return Err(err(StatusCode::FORBIDDEN, "account is deactivated"));
    }

    Ok(AuthUser {
        id: user.id,
        username: user.username,
        role: user.role,
    })
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

    let user = match authenticate(&state, &token).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    request.extensions_mut().insert(user);

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

#[cfg(test)]
mod tests {
    use super::authenticate;
    use uuid::Uuid;

    use crate::auth::{JwtConfig, Role};
    use crate::state::{AppState, NewUser, UpdateUser};

    async fn test_state() -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db_url = format!("sqlite://{}/test.db", tmp.path().display());
        let jwt = JwtConfig::new("test-secret", 3600);
        let state = AppState::new(&db_url, tmp.path().to_path_buf(), jwt)
            .await
            .unwrap();
        (state, tmp)
    }

    #[tokio::test]
    async fn authenticate_requires_active_existing_user() {
        let (state, _tmp) = test_state().await;
        let user = state
            .users
            .create(NewUser {
                id: Uuid::new_v4(),
                username: "alice".into(),
                password_hash: "x".into(),
                role: Role::User,
                display_name: None,
                is_active: true,
                preferred_language: "en".into(),
            })
            .await
            .unwrap();
        let token = state
            .jwt
            .encode(user.id, user.username.clone(), user.role.clone())
            .unwrap();

        // Active account → authenticated.
        let principal = authenticate(&state, &token).await.unwrap();
        assert_eq!(principal.id, user.id);

        // Deactivated account → rejected, even with a still-valid token. This
        // closes the WebDAV/WS bypass that only checked token validity.
        state
            .users
            .update(
                user.id,
                UpdateUser {
                    is_active: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(authenticate(&state, &token).await.is_err());

        // Token for a subject that doesn't exist → rejected.
        let ghost = state
            .jwt
            .encode(Uuid::new_v4(), "ghost".into(), Role::User)
            .unwrap();
        assert!(authenticate(&state, &ghost).await.is_err());
    }
}
