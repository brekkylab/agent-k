use std::{convert::Infallible, sync::Arc};

use aide::NoApi;
use ailoy::{
    agent::{Agent, AgentBuilder, AgentCard},
    message::{Message, MessageOutput, Part, Role},
    runenv::{Sandbox, SandboxConfig},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use chrono::Utc;
use futures_util::StreamExt;
use speedwagon::SpeedwagonSpec;
use uuid::Uuid;

use crate::{
    error::{ApiResult, AppError},
    model::{CreateSessionRequest, SendMessageRequest, SendMessageResponse, SessionResponse},
    state::AppState,
};

const DEFAULT_MODEL: &str = "openai/gpt-4o-mini";

fn sandbox_name_for(id: &Uuid) -> String {
    let s = id.simple().to_string();
    format!("session-{}", &s[..12])
}

async fn build_agent(sandbox: Sandbox) -> Result<Agent, String> {
    let sw_card = AgentCard {
        name: "speedwagon".into(),
        description: "Search the knowledge base for answers. \
            This tool has access to uploaded documents that may contain \
            information the model doesn't have. \
            Use it for any question that could be answered from the knowledge base."
            .into(),
        skills: vec![],
    };
    let sw_spec = SpeedwagonSpec::new().card(sw_card.clone()).into_spec();

    AgentBuilder::new(DEFAULT_MODEL)
        .instruction(concat!(
            "You are a versatile assistant with access to code execution tools ",
            "(bash, python), web search, and a knowledge base (speedwagon). ",
            "You MUST use the speedwagon tool to search the document corpus ",
            "before answering ANY factual question — even if you think you already know the answer. ",
            "The corpus contains authoritative information that may differ from your training data. ",
            "Use bash and python tools for computation, data analysis, and code execution tasks. ",
            "Only skip tools for greetings or casual conversation.",
        ))
        .tool("bash")
        .tool("python_repl")
        .tool("web_search")
        .runenv(sandbox)
        .subagent(sw_spec)
        .build()
        .await
        .map_err(|e| e.to_string())
}

async fn resolve_agent(
    state: &Arc<AppState>,
    id: Uuid,
) -> ApiResult<Arc<tokio::sync::Mutex<Agent>>> {
    if let Some(arc) = state.get_agent(&id) {
        return Ok(arc);
    }

    let session_exists = state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_some();

    if !session_exists {
        return Err(AppError::not_found("session not found"));
    }

    let history = state
        .repository
        .get_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let sandbox_name = sandbox_name_for(&id);
    let cfg = SandboxConfig {
        name: Some(sandbox_name),
        persist: true,
        ..Default::default()
    };
    let sandbox = Sandbox::new(cfg)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let mut agent = build_agent(sandbox)
        .await
        .map_err(|e| AppError::internal(e))?;

    agent.state.history = history;
    tracing::info!(%id, "agent lazy-created with history restored");

    if let Some(existing) = state.get_agent(&id) {
        return Ok(existing);
    }
    state.insert_agent(id, agent);
    Ok(state.get_agent(&id).unwrap())
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(_payload): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let id = Uuid::new_v4();
    let sandbox_name = sandbox_name_for(&id);

    let cfg = SandboxConfig {
        name: Some(sandbox_name.clone()),
        persist: true,
        ..Default::default()
    };
    let sandbox = Sandbox::new(cfg)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let agent = build_agent(sandbox)
        .await
        .map_err(|e| AppError::internal(e))?;

    let now = Utc::now();
    state
        .repository
        .create_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    state.insert_agent(id, agent);

    tracing::info!(%id, sandbox = %sandbox_name, "session created");

    Ok((
        StatusCode::CREATED,
        Json(SessionResponse {
            id,
            created_at: now,
            updated_at: now,
        }),
    ))
}

pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }

    state
        .repository
        .delete_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let agent_arc = state.remove_agent(&id);

    if let Some(arc) = agent_arc {
        drop(arc.lock().await);
        drop(arc);
    }

    let sandbox_name = sandbox_name_for(&id);
    if let Err(e) = ailoy::runenv::remove_persisted(&sandbox_name).await {
        tracing::warn!(%id, "failed to remove persisted sandbox: {e}");
    }

    tracing::info!(%id, "session deleted");
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_message_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<Message>>> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }
    let messages = state
        .repository
        .get_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(messages))
}

pub async fn clear_message_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }
    state
        .repository
        .clear_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    if let Some(arc) = state.get_agent(&id) {
        arc.lock().await.state.history.clear();
    }

    tracing::info!(%id, "message history cleared");
    Ok(StatusCode::NO_CONTENT)
}

pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<Json<SendMessageResponse>> {
    let agent_arc = resolve_agent(&state, id).await?;

    let prev_len = agent_arc.lock().await.get_history().len();

    let outputs = {
        let mut agent = agent_arc.lock().await;
        let msg = Message::new(Role::User).with_contents([Part::text(payload.content)]);
        let mut stream = agent.run(msg);
        let mut outputs: Vec<MessageOutput> = Vec::new();
        while let Some(item) = stream.next().await {
            outputs.push(item.map_err(|e| AppError::internal(e.to_string()))?);
        }
        outputs
    };

    let new_messages = {
        let agent = agent_arc.lock().await;
        agent.get_history()[prev_len..].to_vec()
    };
    state
        .repository
        .append_messages(id, &new_messages)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(outputs))
}

pub async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<
    NoApi<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static>>,
> {
    let agent_arc = resolve_agent(&state, id).await?;
    let repo = state.repository.clone();
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
        drop(run);

        let new_msgs = agent.get_history()[prev_len..].to_vec();
        if let Err(e) = repo.append_messages(id, &new_msgs).await {
            tracing::error!(%id, "failed to persist messages: {e}");
        }

        yield Ok(Event::default().event("done").data("[DONE]"));
    };

    Ok(NoApi(Sse::new(stream).keep_alive(KeepAlive::default())))
}
