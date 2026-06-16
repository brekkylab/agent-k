use std::sync::Arc;

use aide::axum::ApiRouter;

use crate::state::AppState;

mod error;
mod project;
mod session;

pub use project::get_project_router;
pub use session::get_session_router;

pub fn get_router(state: Arc<AppState>) -> ApiRouter {
    ApiRouter::new()
        .merge(get_project_router(state.clone()))
        .merge(get_session_router(state))
}
