use std::sync::Arc;

use aide::axum::{
    ApiRouter,
    routing::{delete, post},
};
use axum::middleware;

use crate::{
    auth::{admin_required, auth_required},
    handlers,
    state::AppState,
};

pub fn get_router(state: Arc<AppState>) -> ApiRouter {
    let auth_routes = ApiRouter::new()
        .api_route("/auth/signup", post(handlers::signup))
        .api_route("/auth/login", post(handlers::login));

    let me_routes = ApiRouter::new()
        .route(
            "/me",
            axum::routing::get(handlers::get_me).patch(handlers::update_me),
        )
        .layer(middleware::from_fn_with_state(state.clone(), auth_required));

    let admin_routes = ApiRouter::new()
        .route(
            "/admin/users",
            axum::routing::get(handlers::list_users).post(handlers::create_user_admin),
        )
        .route(
            "/admin/users/{id}",
            axum::routing::get(handlers::get_user_admin)
                .patch(handlers::update_user_admin)
                .delete(handlers::delete_user_admin),
        )
        .layer(middleware::from_fn(admin_required))
        .layer(middleware::from_fn_with_state(state.clone(), auth_required));

    let session_routes = ApiRouter::new()
        .api_route("/sessions", post(handlers::create_session))
        .api_route("/sessions/{id}", delete(handlers::delete_session))
        .api_route("/sessions/{id}/messages", post(handlers::send_message))
        .route(
            "/sessions/{id}/messages/stream",
            axum::routing::post(handlers::send_message_stream),
        )
        .route(
            "/sessions/{id}/messages",
            axum::routing::get(handlers::get_message_history)
                .delete(handlers::clear_message_history),
        );

    ApiRouter::new()
        .merge(auth_routes)
        .merge(me_routes)
        .merge(admin_routes)
        .merge(session_routes)
        .with_state(state)
}
