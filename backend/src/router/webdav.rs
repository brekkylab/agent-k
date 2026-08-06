use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Response, StatusCode},
    response::IntoResponse,
};
use dav_server::{DavHandler, fakels::FakeLs};
use uuid::Uuid;

use super::cortex_dav::CortexDavFs;
use crate::auth::authenticate;
use crate::state::AppState;

/// WebDAV workspace router. Mounted by [`super::get_router`] at
/// `/workspaces/{wid}/sources[/…]`; serves the **unified** workspace tree —
/// local files under `files/`, each provider mount as a sibling — the same
/// cortex [`Workspace`](cortex::Workspace) the sandbox guest sees.
///
/// Routes via `fallback` so axum forwards every HTTP method — including
/// WebDAV-specific ones (`PROPFIND`, `MKCOL`, `COPY`, `MOVE`, `LOCK`, …) —
/// straight to [`dav_server`]. Authenticated inline (these routes bypass the
/// `auth_required` layer) from the `Authorization: Bearer` header; the token's
/// subject must own the workspace.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new().fallback(handle).with_state(state)
}

async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response<Body> {
    let wid = match parse_wid(req.uri().path()) {
        Some(w) => w,
        None => return (StatusCode::BAD_REQUEST, "invalid workspace id").into_response(),
    };

    // These routes bypass the `auth_required` layer (registered after them), so
    // authenticate inline from the `Authorization: Bearer` header.
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string());
    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
    };
    let user = match authenticate(&state, &token).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    // Access is gated by `get_for_user`: a workspace the caller can't reach is
    // indistinguishable from a missing one (404), so existence can't be probed.
    match state.workspaces.get_for_user(user.id, wid).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "workspace not found").into_response(),
        Err(e) => {
            tracing::error!("workspace lookup failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    }

    // Serve the workspace's cortex [`Workspace`](cortex::Workspace) — the same
    // unified tree (local `files/` + provider mounts) handed to the sandbox
    // guest, wrapped here in the WebDAV protocol by [`CortexDavFs`]. The
    // knowledge resync hook is attached inside `cortex_workspace`, so writes
    // under `files/knowledge/` over WebDAV trigger a resync. Building it loads
    // the workspace's external-provider mounts, which can fail (bad stored
    // config); surface that as a 500 rather than panicking.
    let ws = match state.workspaces.cortex_workspace(wid).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::error!("failed to build workspace filesystem: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    };
    let dav = DavHandler::builder()
        .filesystem(Box::new(CortexDavFs::new(ws)))
        .locksystem(FakeLs::new())
        .strip_prefix(format!("/workspaces/{wid}/sources"))
        .build_handler();

    dav.handle(req).await.map(Body::new)
}

fn parse_wid(path: &str) -> Option<Uuid> {
    let rest = path.strip_prefix("/workspaces/")?;
    let (wid_str, _) = rest.split_once('/')?;
    let wid = Uuid::parse_str(wid_str).ok()?;
    // Require the canonical (lowercase-hyphenated) form. The DAV strip prefix is
    // built from `wid`'s Display and dav-server matches it byte-for-byte, so a
    // non-canonical segment (uppercase, or the 32-char no-hyphen form) would
    // authenticate but then 502 on PrefixMismatch. Reject it up front as a 400.
    (wid_str == wid.to_string()).then_some(wid)
}

#[cfg(test)]
mod tests {
    use super::parse_wid;

    #[test]
    fn parse_wid_requires_canonical_form() {
        let canon = "15fc016b-0000-4000-8000-000000000000";
        // Canonical form, with or without a trailing sub-path.
        assert!(parse_wid(&format!("/workspaces/{canon}/files")).is_some());
        assert!(parse_wid(&format!("/workspaces/{canon}/files/sub/a.txt")).is_some());

        // Uppercase and 32-char no-hyphen forms parse to the same Uuid but
        // aren't canonical — they'd 502 on the byte-exact DAV strip prefix.
        assert!(parse_wid("/workspaces/15FC016B-0000-4000-8000-000000000000/files").is_none());
        assert!(parse_wid("/workspaces/15fc016b000040008000000000000000/files").is_none());

        // Malformed or missing id segment.
        assert!(parse_wid("/workspaces/not-a-uuid/files").is_none());
        assert!(parse_wid(&format!("/workspaces/{canon}")).is_none());
    }
}
