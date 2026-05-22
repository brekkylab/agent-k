use std::sync::Arc;

use aide::axum::{
    ApiRouter,
    routing::{delete, get, post},
};
use axum::handler::Handler;

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
        .api_route("/me", get(handlers::get_me).patch(handlers::update_me))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_required,
        ));

    let admin_routes = ApiRouter::new()
        .api_route(
            "/admin/users",
            get(handlers::list_users).post(handlers::create_user_admin),
        )
        .api_route(
            "/admin/users/{id}",
            get(handlers::get_user_admin)
                .patch(handlers::update_user_admin)
                .delete(handlers::delete_user_admin),
        )
        .layer(axum::middleware::from_fn(admin_required))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_required,
        ));

    let project_routes = ApiRouter::new()
        .api_route(
            "/projects",
            get(handlers::list_projects).post(handlers::create_project),
        )
        .api_route(
            "/projects/{project_id}",
            get(handlers::get_project)
                .patch(handlers::update_project)
                .delete(handlers::delete_project),
        )
        .api_route(
            "/projects/{project_id}/members",
            get(handlers::list_members).post(handlers::add_member),
        )
        .api_route(
            "/projects/{project_id}/members/{user_id}",
            delete(handlers::remove_member),
        )
        .api_route(
            "/projects/{project_id}/sessions",
            get(handlers::list_sessions).post(handlers::create_session),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_required,
        ));

    let dirent_routes = ApiRouter::new()
        .api_route(
            "/dirents",
            post(handlers::dirent_upload.layer(axum::extract::DefaultBodyLimit::disable()))
                .get(handlers::dirent_list)
                .patch(handlers::dirent_batch_op),
        )
        .api_route(
            "/dirents/{*path}",
            get(handlers::dirent_get_file).delete(handlers::dirent_delete),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_required,
        ));

    let session_routes = ApiRouter::new()
        .api_route(
            "/sessions/{session_id}",
            get(handlers::get_session)
                .patch(handlers::update_session)
                .delete(handlers::delete_session),
        )
        .api_route(
            "/sessions/{session_id}/messages",
            get(handlers::get_message_history)
                .post(handlers::send_message)
                .delete(handlers::clear_message_history),
        )
        .api_route("/sessions/{session_id}/fork", post(handlers::fork_session))
        .api_route(
            "/sessions/{session_id}/messages/stream",
            post(handlers::send_message_stream),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_required,
        ));

    let document_routes = ApiRouter::new()
        .api_route(
            "/documents",
            get(handlers::list_documents)
                .post(handlers::ingest_document)
                .delete(handlers::purge_documents),
        )
        .api_route(
            "/documents/{id}",
            get(handlers::get_document).delete(handlers::purge_document),
        );

    // /ws uses plain axum routing: WebSocketUpgrade doesn't implement aide's OperationInput.
    let ws_route = ApiRouter::new()
        .route("/ws", axum::routing::get(handlers::ws_handler))
        .with_state(state.clone());

    ApiRouter::new()
        .merge(auth_routes)
        .merge(me_routes)
        .merge(admin_routes)
        .merge(project_routes)
        .merge(dirent_routes)
        .merge(session_routes)
        .merge(document_routes)
        .merge(ws_route)
        .with_state(state)
}
