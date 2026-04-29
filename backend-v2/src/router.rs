use std::convert::Infallible;
use std::sync::Arc;

use aide::axum::{
    ApiRouter,
    routing::{delete, post},
};
use ailoy::{
    agent::AgentBuilder,
    lang_model::{LangModel, LangModelProvider},
    message::{Message, MessageOutput, Part, Role},
    runenv::{Sandbox, SandboxConfig},
    tool::{BuiltinToolProvider, make_builtin_tool},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use chrono::Utc;
use futures_util::StreamExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    error::AppError,
    model::{CreateSessionRequest, SendMessageRequest, SendMessageResponse, Session},
    state::AppState,
};

const DEFAULT_MODEL: &str = "anthropic/claude-haiku-4-5-20251001";

fn sandbox_name_for(id: &Uuid) -> String {
    let s = id.simple().to_string();
    format!("session-{}", &s[..12])
}

pub fn get_router(state: Arc<Mutex<AppState>>) -> ApiRouter {
    ApiRouter::new()
        .api_route("/sessions", post(create_session))
        .api_route("/sessions/{id}", delete(delete_session))
        .api_route("/sessions/{id}/messages", post(send_message))
        .route(
            "/sessions/{id}/messages/stream",
            axum::routing::post(send_message_stream),
        )
        .route(
            "/sessions/{id}/messages",
            axum::routing::get(get_message_history).delete(clear_message_history),
        )
        .with_state(state)
}

async fn create_session(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(_payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), (StatusCode, Json<AppError>)> {
    let id = Uuid::new_v4();
    let sandbox_name = sandbox_name_for(&id);

    let cfg = SandboxConfig {
        name: Some(sandbox_name.clone()),
        persist: true,
        ..Default::default()
    };
    let sandbox = Arc::new(Sandbox::new(cfg).await.map_err(internal)?);

    let agent = build_agent(sandbox).await.map_err(internal)?;

    let now = Utc::now();

    {
        let mut st = state.lock().await;
        st.repository.create_session(id).await.map_err(internal)?;
        st.insert_agent(id, agent);
    }

    tracing::info!(%id, sandbox = %sandbox_name, "session created");

    Ok((
        StatusCode::CREATED,
        Json(Session {
            id,
            created_at: now,
            updated_at: now,
        }),
    ))
}

async fn delete_session(
    State(state): State<Arc<Mutex<AppState>>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
    let agent_arc = {
        let mut st = state.lock().await;

        // Verify session exists in DB (covers the in-memory-less case too).
        if st
            .repository
            .get_session(id)
            .await
            .map_err(internal)?
            .is_none()
        {
            return Err((
                StatusCode::NOT_FOUND,
                Json(AppError::new("session not found")),
            ));
        }

        st.repository.delete_session(id).await.map_err(internal)?;
        st.remove_agent(&id)
    };

    // Wait for any in-progress run before dropping.
    if let Some(arc) = agent_arc {
        drop(arc.lock().await);
        drop(arc);
    }

    let name = sandbox_name_for(&id);
    ailoy::runenv::remove_persisted(&name)
        .await
        .map_err(internal)?;

    tracing::info!(%id, sandbox = %name, "session deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// Resolve or lazy-create the in-memory agent for `id`.
///
/// On the first request after a server restart the agent is not in memory but
/// the session and its message history are in the DB. This function rebuilds
/// the agent and restores the history so the next turn starts with full context.
async fn resolve_agent(
    state: &Arc<Mutex<AppState>>,
    id: Uuid,
) -> Result<Arc<Mutex<ailoy::agent::Agent>>, (StatusCode, Json<AppError>)> {
    // Fast path: agent already in memory.
    {
        let st = state.lock().await;
        if let Some(arc) = st.get_agent(&id) {
            return Ok(arc);
        }
    }

    // Slow path: session must exist in DB.
    let (session_exists, history, repo) = {
        let st = state.lock().await;
        let exists = st
            .repository
            .get_session(id)
            .await
            .map_err(internal)?
            .is_some();
        let history = if exists {
            st.repository.get_messages(id).await.map_err(internal)?
        } else {
            vec![]
        };
        (exists, history, st.repository.clone())
    };

    if !session_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(AppError::new("session not found")),
        ));
    }

    // Build agent outside the mutex (async I/O).
    let sandbox_name = sandbox_name_for(&id);
    let cfg = SandboxConfig {
        name: Some(sandbox_name.clone()),
        persist: true,
        ..Default::default()
    };
    let sandbox = Arc::new(Sandbox::new(cfg).await.map_err(internal)?);
    let mut agent = build_agent(sandbox).await.map_err(internal)?;

    // Restore persisted history so the agent has full conversation context.
    agent.state.history = history;

    tracing::info!(%id, sandbox = %sandbox_name, "agent lazy-created with history restored");

    let _ = repo; // repo clone kept alive until here

    // Insert — if another request won the race, use theirs.
    let mut st = state.lock().await;
    if let Some(existing) = st.get_agent(&id) {
        return Ok(existing);
    }
    st.insert_agent(id, agent);
    Ok(st.get_agent(&id).unwrap())
}

async fn get_message_history(
    State(state): State<Arc<Mutex<AppState>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Message>>, (StatusCode, Json<AppError>)> {
    let st = state.lock().await;
    if st
        .repository
        .get_session(id)
        .await
        .map_err(internal)?
        .is_none()
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(AppError::new("session not found")),
        ));
    }
    let messages = st.repository.get_messages(id).await.map_err(internal)?;
    Ok(Json(messages))
}

async fn clear_message_history(
    State(state): State<Arc<Mutex<AppState>>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
    let agent_arc = {
        let st = state.lock().await;
        if st
            .repository
            .get_session(id)
            .await
            .map_err(internal)?
            .is_none()
        {
            return Err((
                StatusCode::NOT_FOUND,
                Json(AppError::new("session not found")),
            ));
        }
        st.repository.clear_messages(id).await.map_err(internal)?;
        st.get_agent(&id)
    };

    if let Some(arc) = agent_arc {
        arc.lock().await.state.history.clear();
    }

    tracing::info!(%id, "message history cleared");
    Ok(StatusCode::NO_CONTENT)
}

async fn send_message(
    State(state): State<Arc<Mutex<AppState>>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, (StatusCode, Json<AppError>)> {
    let agent_arc = resolve_agent(&state, id).await?;

    let prev_len = agent_arc.lock().await.get_history().len();

    let outputs = {
        let mut agent = agent_arc.lock().await;
        let msg = Message::new(Role::User).with_contents([Part::text(payload.content)]);
        let mut stream = agent.run(msg);
        let mut outputs: Vec<MessageOutput> = Vec::new();
        while let Some(item) = stream.next().await {
            outputs.push(item.map_err(internal)?);
        }
        outputs
    };

    // Persist newly added history entries.
    let new_messages = {
        let agent = agent_arc.lock().await;
        agent.get_history()[prev_len..].to_vec()
    };
    state
        .lock()
        .await
        .repository
        .append_messages(id, &new_messages)
        .await
        .map_err(internal)?;

    Ok(Json(outputs))
}

async fn send_message_stream(
    State(state): State<Arc<Mutex<AppState>>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static>,
    (StatusCode, Json<AppError>),
> {
    let agent_arc = resolve_agent(&state, id).await?;
    let repo = state.lock().await.repository.clone();
    let prev_len = agent_arc.lock().await.get_history().len();
    let content = payload.content;

    let stream = async_stream::stream! {
        let mut agent = agent_arc.lock().await;
        let msg = Message::new(Role::User).with_contents([Part::text(content)]);
        let mut run = agent.run(msg);

        while let Some(item) = run.next().await {
            match item {
                Ok(output) => {
                    let json = serde_json::to_string(&output)
                        .unwrap_or_else(|e| format!("{{\"error\":\"{e}}}", e = e));
                    yield Ok::<Event, Infallible>(
                        Event::default().event("message").data(json),
                    );
                }
                Err(e) => {
                    yield Ok(Event::default().event("error").data(e.to_string()));
                    return;
                }
            }
        }
        // Drop the mutable stream borrow before taking an immutable borrow of history.
        drop(run);

        // Persist after stream is fully consumed.
        let new_msgs = agent.get_history()[prev_len..].to_vec();
        if let Err(e) = repo.append_messages(id, &new_msgs).await {
            tracing::error!(%id, "failed to persist messages: {e}");
        }

        yield Ok(Event::default().event("done").data("[DONE]"));
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn build_agent(sandbox: Arc<ailoy::runenv::Sandbox>) -> Result<ailoy::agent::Agent, String> {
    let (bash, python, web_search) = tokio::try_join!(
        make_builtin_tool(&BuiltinToolProvider::Bash {}),
        make_builtin_tool(&BuiltinToolProvider::PythonRepl {}),
        make_builtin_tool(&BuiltinToolProvider::WebSearch {}),
    )
    .map_err(|e| e.to_string())?;
    let model = build_lang_model(DEFAULT_MODEL)?;
    AgentBuilder::new(model)
        .tool(bash)
        .tool(python)
        .tool(web_search)
        .runenv(sandbox)
        .build()
        .await
        .map_err(|e| e.to_string())
}

fn build_lang_model(model_full_id: &str) -> Result<LangModel, String> {
    if let Some(m) = model_full_id.strip_prefix("anthropic/") {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;
        Ok(LangModel::new(
            m.to_string(),
            LangModelProvider::anthropic(key),
        ))
    } else if let Some(m) = model_full_id.strip_prefix("openai/") {
        let key =
            std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set".to_string())?;
        Ok(LangModel::new(
            m.to_string(),
            LangModelProvider::openai(key),
        ))
    } else if let Some(m) = model_full_id.strip_prefix("google/") {
        let key =
            std::env::var("GEMINI_API_KEY").map_err(|_| "GEMINI_API_KEY not set".to_string())?;
        Ok(LangModel::new(
            m.to_string(),
            LangModelProvider::gemini(key),
        ))
    } else {
        Err(format!(
            "unknown provider prefix in model id: {}",
            model_full_id
        ))
    }
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, Json<AppError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(AppError::new(e.to_string())),
    )
}
