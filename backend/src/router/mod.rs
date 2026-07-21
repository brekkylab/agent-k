use std::sync::Arc;

use aide::axum::{
    ApiRouter,
    routing::{delete, get, post},
};

use crate::{
    auth::{admin_required, auth_required},
    state::AppState,
};

pub(crate) mod error;

mod agent;
mod auth;
mod knowledge;
mod message;
mod mount;
mod session;
mod user;
mod webdav;
mod workspace;

pub fn get_router(state: Arc<AppState>) -> ApiRouter {
    // Layering rules:
    // - `admin_required` is applied first to the `/admin/*` routes, so it
    //   sits closest to those handlers.
    // - `auth_required` is then applied to every route registered above it
    //   (admin + `/me` + workspaces + sessions HTTP), wrapping them as the
    //   outer layer so `AuthUser` is populated before `admin_required`
    //   inspects the role.
    // - Public routes (`/auth/*`, the WS endpoint) are registered after
    //   both layers and therefore bypass them. The WS endpoint authenticates
    //   inline via a `?token=…` query parameter because browser `WebSocket`
    //   clients can't send custom `Authorization` headers.
    ApiRouter::new()
        .api_route(
            "/admin/users",
            get(user::list_users).post(user::create_user_admin),
        )
        .api_route(
            "/admin/users/{id}",
            get(user::get_user_admin)
                .patch(user::update_user_admin)
                .delete(user::delete_user_admin),
        )
        .route_layer(axum::middleware::from_fn(admin_required))
        .api_route("/me", get(user::get_me).patch(user::update_me))
        .api_route(
            "/me/workspace",
            get(workspace::get_my_workspace).patch(workspace::update_my_workspace),
        )
        // `/workspaces` list unwired: with one workspace per user it only ever
        // returns the default, which /me/workspace already serves. Re-add (as a
        // real list) with multiple workspaces per user:
        // .api_route("/workspaces", get(workspace::list_workspaces))
        .api_route(
            "/workspaces/{id}",
            get(workspace::get_workspace)
                .patch(workspace::update_workspace),
            // DELETE unwired until multiple workspaces per user exist — today it
            // would only ever 403 (default) or 404 (see delete_workspace):
            // .delete(workspace::delete_workspace)
        )
        .api_route(
            "/workspaces/{wid}/mounts",
            get(mount::list_mounts).post(mount::create_mount),
        )
        .api_route(
            "/workspaces/{wid}/mounts/{mount_id}",
            delete(mount::delete_mount),
        )
        .api_route(
            "/workspaces/{wid}/knowledge/refs",
            get(knowledge::list_refs).post(knowledge::create_ref),
        )
        .api_route(
            "/workspaces/{wid}/knowledge/refs/{name}",
            delete(knowledge::delete_ref),
        )
        .api_route(
            "/workspaces/{wid}/knowledge/resync",
            post(knowledge::resync),
        )
        .api_route(
            "/agents",
            get(agent::list_agents).post(agent::create_agent),
        )
        .api_route(
            "/agents/{id}",
            get(agent::get_agent)
                .patch(agent::update_agent)
                .delete(agent::delete_agent),
        )
        .api_route(
            "/sessions",
            get(session::list_sessions).post(session::create_session),
        )
        .api_route(
            "/sessions/{id}",
            get(session::get_session).delete(session::delete_session),
        )
        .api_route(
            "/sessions/{id}/messages",
            get(message::list_messages).post(message::start_run),
        )
        .api_route("/sessions/{id}/messages/stop", post(message::stop_run))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_required,
        ))
        .api_route("/auth/signup", post(auth::signup))
        .api_route("/auth/login", post(auth::login))
        .route(
            "/sessions/{id}/messages/ws",
            axum::routing::get(message::stream_messages),
        )
        // WebDAV serves the unified workspace tree (local under `files/`, each
        // provider mount as a sibling) at `/sources`. Two routes: matchit's
        // `{*rest}` wildcard needs one-or-more segments, so the bare collection
        // root (`/…/sources`) needs its own entry — without it, a root
        // `PROPFIND` 404s.
        .route_service("/workspaces/{wid}/sources", webdav::router(state.clone()))
        .route_service(
            "/workspaces/{wid}/sources/{*rest}",
            webdav::router(state.clone()),
        )
        .with_state(state)
}
