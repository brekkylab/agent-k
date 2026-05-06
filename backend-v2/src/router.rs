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
    let public_routes = ApiRouter::new()
        .api_route("/auth/signup", post(handlers::auth::signup))
        .api_route("/auth/login", post(handlers::auth::login));

    let me_routes = ApiRouter::new()
        .route(
            "/me",
            axum::routing::get(handlers::user::get_me).patch(handlers::user::update_me),
        )
        .layer(middleware::from_fn_with_state(state.clone(), auth_required));

    let admin_routes = ApiRouter::new()
        .route(
            "/admin/users",
            axum::routing::get(handlers::user::list_users)
                .post(handlers::user::create_user_admin),
        )
        .route(
            "/admin/users/{id}",
            axum::routing::get(handlers::user::get_user_admin)
                .patch(handlers::user::update_user_admin)
                .delete(handlers::user::delete_user_admin),
        )
        .layer(middleware::from_fn(admin_required))
        .layer(middleware::from_fn_with_state(state.clone(), auth_required));

    let session_routes = ApiRouter::new()
        .api_route("/sessions", post(handlers::session::create_session))
        .api_route("/sessions/{id}", delete(handlers::session::delete_session))
        .api_route(
            "/sessions/{id}/messages",
            post(handlers::session::send_message),
        )
        .route(
            "/sessions/{id}/messages/stream",
            axum::routing::post(handlers::session::send_message_stream),
        )
        .route(
            "/sessions/{id}/messages",
            axum::routing::get(handlers::session::get_message_history)
                .delete(handlers::session::clear_message_history),
        );

    ApiRouter::new()
        .merge(public_routes)
        .merge(me_routes)
        .merge(admin_routes)
        .merge(session_routes)
        .with_state(state)
}
