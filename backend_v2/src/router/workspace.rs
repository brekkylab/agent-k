use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Response, StatusCode},
    response::IntoResponse,
};
use dav_server::{DavHandler, fakels::FakeLs, localfs::LocalFs};
use uuid::Uuid;

use crate::state::AppState;

/// WebDAV workspace router. Mounted by [`super::get_router`] at
/// `/projects/{pid}/workspace[/…]`; exposes `data_root/{pid}/workspace` as a
/// per-project filesystem.
///
/// Routes via `fallback` so axum forwards every HTTP method — including
/// WebDAV-specific ones (`PROPFIND`, `MKCOL`, `COPY`, `MOVE`, `LOCK`, …) —
/// straight to [`dav_server`]. Auth mirrors the WS route: JWT is read from
/// `?token=…` because the eventual target audience (browser fetch + native
/// WebDAV clients) cannot reliably set custom auth headers.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new().fallback(handle).with_state(state)
}

async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response<Body> {
    let pid = match parse_pid(req.uri().path()) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "invalid project id").into_response(),
    };

    let token = req.uri().query().and_then(extract_token);
    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "missing token").into_response();
    };
    if state.jwt.decode(&token).is_err() {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }

    match state.projects.get(pid).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "project not found").into_response(),
        Err(e) => {
            tracing::error!("workspace project lookup failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    }

    let root = state.projects.workspace_root(pid);
    if let Err(e) = tokio::fs::create_dir_all(&root).await {
        tracing::error!("workspace mkdir failed: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "mkdir failed").into_response();
    }

    let dav = DavHandler::builder()
        .filesystem(LocalFs::new(&root, false, false, false))
        .locksystem(FakeLs::new())
        .strip_prefix(format!("/projects/{pid}/workspace"))
        .build_handler();

    dav.handle(req).await.map(Body::new)
}

fn parse_pid(path: &str) -> Option<Uuid> {
    let rest = path.strip_prefix("/projects/")?;
    let (pid_str, _) = rest.split_once('/')?;
    Uuid::parse_str(pid_str).ok()
}

fn extract_token(query: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v.into_owned())
}
