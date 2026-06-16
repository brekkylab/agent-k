use std::sync::Arc;

use aide::axum::ApiRouter;

use crate::state::AppState;

pub(crate) mod error;

mod auth;
mod project;
mod session;
mod user;

pub use auth::get_auth_router;
pub use project::get_project_router;
pub use session::get_session_router;
pub use user::get_user_router;

pub fn get_router(state: Arc<AppState>) -> ApiRouter {
    ApiRouter::new()
        .merge(get_auth_router(state.clone()))
        .merge(get_user_router(state.clone()))
        .merge(get_project_router(state.clone()))
        .merge(get_session_router(state.clone()))
}
