use std::pin::Pin;

use ailoy::{AgentProvider, LangModelAPISchema, LangModelProvider};
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chat_agent::ChatEvent;
use futures_util::StreamExt;
use uuid::Uuid;

use crate::models::{
    CreateSessionRequest, ErrorResponse, MessageRole, Session, SessionDetailResponse,
    SessionMessage, SessionToolCall, UpdateSessionRequest,
};
use crate::repository::RepositoryError;
use crate::state::AppState;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session not found")]
    NotFound,
    #[error("agent not found")]
    AgentNotFound,
    #[error("provider profile not found")]
    ProviderProfileNotFound,
    #[error("no default provider profile available")]
    NoDefaultProviderProfile,
    #[error("message content is empty")]
    EmptyContent,
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("runtime error: {0}")]
    Runtime(String),
}

impl IntoResponse for SessionError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::NotFound | Self::AgentNotFound | Self::ProviderProfileNotFound => {
                StatusCode::NOT_FOUND
            }
            Self::NoDefaultProviderProfile | Self::EmptyContent => StatusCode::BAD_REQUEST,
            Self::Runtime(_) => StatusCode::BAD_GATEWAY,
            Self::Repository(e) => {
                tracing::error!("repository error: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let error_msg = if matches!(self, Self::Repository(_)) {
            "internal server error".to_string()
        } else {
            self.to_string()
        };
        (status, Json(ErrorResponse { error: error_msg })).into_response()
    }
}

/// Load a session with its messages and attached tool calls, assembled into SessionDetailResponse.
pub async fn get_session_detail(
    state: &AppState,
    id: Uuid,
) -> Result<Option<SessionDetailResponse>, SessionError> {
    let Some(session) = state.repository.get_session(id).await? else {
        return Ok(None);
    };
    let tool_calls = state
        .repository
        .get_tool_calls_for_session(id)
        .await
        .unwrap_or_default();
    Ok(Some(SessionDetailResponse::from((session, tool_calls))))
}

/// Create a new session after verifying the agent exists and resolving the provider profile.
pub async fn create_session(
    state: &AppState,
    req: CreateSessionRequest,
) -> Result<Session, SessionError> {
    let CreateSessionRequest {
        agent_id,
        provider_profile_id,
        title,
        speedwagon_ids,
        source_ids,
    } = req;

    // Verify agent exists
    match state.repository.get_agent(agent_id).await? {
        Some(_) => {}
        None => return Err(SessionError::AgentNotFound),
    }

    // Resolve provider profile
    let resolved_id = resolve_provider_profile_id(state, provider_profile_id).await?;

    let session = state
        .repository
        .create_session(agent_id, resolved_id, title, speedwagon_ids, source_ids)
        .await?;

    Ok(session)
}

/// Update a session atomically (title, agent, provider, speedwagons, sources) and invalidate runtime cache.
pub async fn update_session(
    state: &AppState,
    id: Uuid,
    req: UpdateSessionRequest,
) -> Result<Session, SessionError> {
    // Validate agent if provided
    if let Some(agent_id) = req.agent_id {
        match state.repository.get_agent(agent_id).await? {
            Some(_) => {}
            None => return Err(SessionError::AgentNotFound),
        }
    }

    // Validate provider profile if provided
    if let Some(pp_id) = req.provider_profile_id {
        match state.repository.get_provider_profile(pp_id).await? {
            Some(_) => {}
            None => return Err(SessionError::ProviderProfileNotFound),
        }
    }

    // Atomic update via transaction
    let (session, previous_agent_id) = state
        .repository
        .update_session_atomic(
            id,
            req.title,
            req.agent_id,
            req.provider_profile_id,
            req.speedwagon_ids,
            req.source_ids,
        )
        .await?
        .ok_or(SessionError::NotFound)?;

    // Invalidate cache once after successful commit
    state.invalidate_session_runtime(id);

    // CoW symmetry: if the agent pointer was swapped, the previous agent may now be
    // orphaned. Mirror delete_session's cleanup so immutable agents don't accumulate.
    if let Some(prev) = previous_agent_id {
        super::agent::cleanup_orphaned_agent(state, prev).await;
    }

    Ok(session)
}

/// Delete a session, invalidate its runtime cache, and cascade-delete orphaned agent.
pub async fn delete_session(state: &AppState, id: Uuid) -> Result<(), SessionError> {
    // Fetch session first to remember agent_id for potential orphan cleanup
    let session = state
        .repository
        .get_session(id)
        .await?
        .ok_or(SessionError::NotFound)?;
    let agent_id = session.agent_id;

    let deleted = state.repository.delete_session(id).await?;
    if !deleted {
        return Err(SessionError::NotFound);
    }
    state.invalidate_session_runtime(id);

    // Best-effort: clean up agent if no sessions reference it anymore
    super::agent::cleanup_orphaned_agent(state, agent_id).await;

    Ok(())
}

/// SSE event types emitted during streaming message processing.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    Thinking {
        level: String,
    },
    ToolCall {
        level: String,
        tool: String,
        args: Option<serde_json::Value>,
    },
    ToolResult {
        level: String,
        tool: String,
        result: Option<serde_json::Value>,
        error: Option<String>,
    },
    Message {
        level: String,
        content: String,
    },
    Done {
        assistant_message: SessionMessage,
    },
    Error {
        message: String,
    },
}

/// Save user message, run ChatAgent in streaming mode, and yield SSE events.
/// The final `Done` event includes the persisted assistant message.
///
/// Uses an mpsc channel: a spawned task (inside a LocalSet) holds the MutexGuard
/// and streams ChatEvents, sending SSE events through the channel.
/// This avoids the `Send` bound issue since `run_user_text_streaming` returns a `!Send` stream.
pub async fn send_message_streaming(
    state: &AppState,
    session_id: Uuid,
    content: String,
) -> Result<
    Pin<Box<dyn futures_util::Stream<Item = Result<SseEvent, SessionError>> + Send>>,
    SessionError,
> {
    if content.trim().is_empty() {
        return Err(SessionError::EmptyContent);
    }

    // Save the user message
    state
        .repository
        .add_session_message(session_id, MessageRole::User, content.clone())
        .await?
        .ok_or(SessionError::NotFound)?;

    // Load session for runtime creation
    let session = state
        .repository
        .get_session(session_id)
        .await?
        .ok_or(SessionError::NotFound)?;

    // Get or create runtime
    let runtime = state
        .get_or_create_runtime_for_session(&session)
        .await
        .map_err(|e| SessionError::Runtime(format!("failed to initialize agent runtime: {e}")))?;

    let repository = state.repository.clone();

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, SessionError>>(32);

    // `run_user_text_streaming` returns a `!Send` stream, so we use spawn_blocking
    // to run it inside a new tokio runtime on a dedicated thread.
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build local runtime");
        let local_set = tokio::task::LocalSet::new();
        local_set.block_on(&rt, async move {
            let mut rt = runtime.lock().await;
            let mut event_stream = rt.run_user_text_streaming(content);

            let mut assistant_content: Option<String> = None;
            let mut tool_calls_for_db: Vec<(
                String,
                Option<serde_json::Value>,
                Option<serde_json::Value>,
            )> = Vec::new();

            while let Some(event_result) = event_stream.next().await {
                let sse = match event_result {
                    Ok(ChatEvent::Thinking) => SseEvent::Thinking {
                        level: "info".to_string(),
                    },
                    Ok(ChatEvent::ToolCall { tool, args }) => {
                        tool_calls_for_db.push((tool.clone(), Some(args.clone()), None));
                        SseEvent::ToolCall {
                            level: "info".to_string(),
                            tool,
                            args: Some(args),
                        }
                    }
                    Ok(ChatEvent::ToolResult { tool, result }) => {
                        // Update the matching tool call's result (last one without a result)
                        if let Some(tc) = tool_calls_for_db
                            .iter_mut()
                            .rev()
                            .find(|(n, _, r)| n == &tool && r.is_none())
                        {
                            tc.2 = Some(result.clone());
                        }
                        SseEvent::ToolResult {
                            level: "info".to_string(),
                            tool,
                            result: Some(result),
                            error: None,
                        }
                    }
                    Ok(ChatEvent::Message {
                        content,
                        tool_calls: _,
                    }) => {
                        assistant_content = Some(content.clone());
                        SseEvent::Message {
                            level: "info".to_string(),
                            content,
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Ok(SseEvent::Error {
                                message: e.to_string(),
                            }))
                            .await;
                        return;
                    }
                };
                if tx.send(Ok(sse)).await.is_err() {
                    return; // receiver dropped
                }
            }

            // Trim history to 20 turns after streaming completes
            drop(event_stream);
            rt.trim_history();
            drop(rt);

            // Save assistant message + tool calls to DB
            if let Some(content) = &assistant_content {
                match repository
                    .add_session_message(session_id, MessageRole::Assistant, content.clone())
                    .await
                {
                    Ok(Some(msg)) => {
                        // Save tool calls
                        if !tool_calls_for_db.is_empty() {
                            let tool_call_models: Vec<SessionToolCall> = tool_calls_for_db
                                .iter()
                                .map(|(name, args, result)| SessionToolCall {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    message_id: msg.id.clone(),
                                    tool_name: name.clone(),
                                    tool_args: args.clone(),
                                    tool_result: result.clone(),
                                    duration_ms: None,
                                    created_at: chrono::Utc::now(),
                                })
                                .collect();
                            let _ = repository.save_tool_calls(&msg.id, &tool_call_models).await;
                        }
                        let _ = tx
                            .send(Ok(SseEvent::Done {
                                assistant_message: msg,
                            }))
                            .await;
                    }
                    Ok(None) => {
                        let _ = tx
                            .send(Ok(SseEvent::Error {
                                message: "session not found".to_string(),
                            }))
                            .await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Ok(SseEvent::Error {
                                message: format!("failed to save message: {e}"),
                            }))
                            .await;
                    }
                }
            } else {
                let _ = tx
                    .send(Ok(SseEvent::Error {
                        message: "no assistant response".to_string(),
                    }))
                    .await;
            }
        });
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(Box::pin(stream))
}

async fn resolve_provider_profile_id(
    state: &AppState,
    requested: Option<Uuid>,
) -> Result<Uuid, SessionError> {
    if let Some(id) = requested {
        return match state.repository.get_provider_profile(id).await? {
            Some(_) => Ok(id),
            None => Err(SessionError::ProviderProfileNotFound),
        };
    }

    let profiles = state.repository.list_provider_profiles().await?;

    profiles
        .iter()
        .filter(|p| p.is_default)
        .min_by(|a, b| {
            provider_priority(&a.provider)
                .cmp(&provider_priority(&b.provider))
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.id.cmp(&b.id))
        })
        .map(|p| p.id)
        .ok_or(SessionError::NoDefaultProviderProfile)
}

fn provider_priority(provider: &AgentProvider) -> u8 {
    match &provider.lm {
        LangModelProvider::API { schema, .. } => match schema {
            LangModelAPISchema::ChatCompletion => 0,
            LangModelAPISchema::Anthropic => 1,
            LangModelAPISchema::Gemini => 2,
            LangModelAPISchema::OpenAI => 3,
        },
    }
}
