use std::sync::Arc;

use aide::axum::{ApiRouter, routing::post};
use ailoy::agent::{Agent, AgentSpec, default_provider};
use axum::{Json, extract::State, http::StatusCode};
use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    error::AppError,
    model::{CreateSessionRequest, SessionResponse},
    state::AppState,
};

pub fn get_router(state: Arc<Mutex<AppState>>) -> ApiRouter {
    ApiRouter::new()
        .api_route("/sessions", post(create_session))
        .with_state(state)
}

async fn create_session(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(_payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), (StatusCode, Json<AppError>)> {
    // @jhlee: Currently it is hard-coded. We may use `/agent` entity later.
    let spec = AgentSpec::new("openai/gpt-4.5-mini");

    // @jhlee: TODO build speedwagon toolset
    // @jhlee: TODO make ToolSet global static
    // let ts = speedwagon::build_toolset(store);
    let ts = ailoy::tool::ToolSet::new();

    // Agent::try_new produces a !Send future due to ToolFactory in ailoy;
    // run it in a blocking thread so the handler future stays Send.
    let agent = Agent::try_with_tools(spec, &*default_provider().await, &ts)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(AppError::new(e.to_string())),
            )
        })?;

    let id = Uuid::new_v4();
    let now = Utc::now();
    state.lock().await.insert_agent(id, agent);

    Ok((
        StatusCode::CREATED,
        Json(SessionResponse {
            id,
            created_at: now,
            updated_at: now,
        }),
    ))
}

// async fn list_sessions(
//     State(state): State<AppState_>,
//     Query(query): Query<ListSessionsQuery>,
// ) -> Result<Json<Vec<SessionResponse>>, (StatusCode, Json<AppError>)> {
//     let ListSessionsQuery {
//         agent_id,
//         include_messages,
//     } = query;

//     let sessions = state
//         .repository
//         .list_sessions(agent_id, include_messages.unwrap_or(false))
//         .await
//         .map_err(repo_err)?;
//     Ok(Json(sessions.iter().map(SessionResponse::from).collect()))
// }

// async fn get_session(
//     State(state): State<AppState_>,
//     Path(id): Path<Uuid>,
// ) -> ApiResult<Json<SessionDetailResponse>> {
//     match session_service::get_session_detail(&state, id)
//         .await
//         .map_err(session_err)?
//     {
//         Some(detail) => Ok(Json(detail)),
//         None => Err(AppError::not_found("session not found")),
//     }
// }

// async fn update_session(
//     State(state): State<AppState_>,
//     Path(id): Path<Uuid>,
//     Json(payload): Json<UpdateSessionRequest>,
// ) -> Result<Json<SessionResponse>, (StatusCode, Json<AppError>)> {
//     let session = session_service::update_session(&state, id, payload)
//         .await
//         .map_err(|e| AppError::internal(e.to_string()))?;
//     Ok(Json(SessionResponse::from(&session)))
// }

// async fn delete_session(
//     State(state): State<AppState_>,
//     Path(id): Path<Uuid>,
// ) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
//     session_service::delete_session(&state, id)
//         .await
//         .map_err(|e| AppError::internal(e.to_string()))?;
//     Ok(StatusCode::NO_CONTENT)
// }
