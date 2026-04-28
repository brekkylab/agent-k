use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use aide::axum::{ApiRouter, routing::post};
use ailoy::agent::{Agent, AgentBuilder, AgentCard, default_provider};
use ailoy::lang_model::LangModel;
use ailoy::message::{Message, Part, Role};
use ailoy::tool::ToolSet;
use axum::extract::Path;
use axum::{Json, extract::State, http::StatusCode};
use chrono::Utc;
use futures_util::StreamExt;
use speedwagon::{SharedStore, SpeedwagonSpec, Store, build_toolset};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, AppError},
    model::{CreateSessionRequest, SendMessageRequest, SendMessageResponse, SessionResponse},
    state::AppState,
};

static STORE: OnceLock<SharedStore> = OnceLock::new();
static TOOLSET: OnceLock<ToolSet> = OnceLock::new();

pub fn speedwagon_store() -> SharedStore {
    STORE
        .get_or_init(|| {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".speedwagon");
            Arc::new(RwLock::new(
                Store::new(path).expect("speedwagon store init"),
            ))
        })
        .clone()
}

fn speedwagon_toolset() -> &'static ToolSet {
    TOOLSET.get_or_init(|| build_toolset(speedwagon_store()))
}

pub fn get_router(state: Arc<AppState>) -> ApiRouter {
    ApiRouter::new()
        .api_route("/sessions", post(create_session))
        .api_route("/sessions/{id}/messages", post(send_message))
        .with_state(state)
}

const MESSAGE_TIMEOUT: Duration = Duration::from_secs(120);

async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<Json<SendMessageResponse>> {
    let agent_handle = state
        .get_agent(&session_id)
        .ok_or_else(|| AppError::not_found(format!("session '{session_id}' not found")))?;

    let query = Message::new(Role::User).with_contents([Part::text(&payload.content)]);

    let result = tokio::time::timeout(MESSAGE_TIMEOUT, async {
        let mut agent = agent_handle.lock().await;
        let mut stream = agent.run(query);
        let mut messages: Vec<Message> = Vec::new();
        let mut final_content = String::new();

        while let Some(result) = stream.next().await {
            let output = result.map_err(|e| AppError::internal(e.to_string()))?;
            if output.message.role == Role::Assistant {
                for part in &output.message.contents {
                    if let Some(text) = part.as_text() {
                        final_content = text.to_string();
                    }
                }
            }
            messages.push(output.message);
        }

        Ok::<_, ApiError>(SendMessageResponse {
            messages,
            final_content,
        })
    })
    .await;

    match result {
        Ok(inner) => Ok(Json(inner?)),
        Err(_elapsed) => Err(AppError::internal("agent response timed out")),
    }
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(_payload): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let provider_guard = default_provider().await;

    let sw_card = AgentCard {
        name: "speedwagon".into(),
        description: "Search a local document corpus and read passages. \
            Use when the user asks about documents, facts, quotes, citations, \
            or computations on document data."
            .into(),
        skills: vec![],
    };
    let sw_spec = SpeedwagonSpec::new().card(sw_card.clone()).into_spec();
    let sw_agent = Agent::try_with_tools(sw_spec, &*provider_guard, speedwagon_toolset())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let main_model_name = "openai/gpt-4.5-mini";
    let model_id = main_model_name
        .split_once('/')
        .map(|(_, id)| id)
        .unwrap_or(main_model_name);
    let model_provider = provider_guard
        .get_model(main_model_name)
        .ok_or_else(|| AppError::internal(format!("No provider for model '{main_model_name}'")))?
        .clone();
    let model = LangModel::new(model_id.to_string(), model_provider);
    drop(provider_guard);

    let agent = AgentBuilder::new(model)
        .subagent(sw_card, sw_agent)
        .build()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    // Alternative: Speedwagon을 main agent로 직접 사용 (subagent 패턴 없이)
    // let spec = SpeedwagonSpec::new().model(main_model_name).card(sw_card).into_spec();
    // let agent = Agent::try_with_tools(spec, &*provider_guard, speedwagon_toolset())
    //     .await
    //     .map_err(|e| AppError::internal(e.to_string()))?;
    // drop(provider_guard);

    let id = Uuid::new_v4();
    let now = Utc::now();
    state.insert_agent(id, agent);

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
