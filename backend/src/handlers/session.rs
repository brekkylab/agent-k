use std::{convert::Infallible, sync::Arc};

use agent_k::agents::SpeedwagonSpec;
use aide::NoApi;
use ailoy::{
    agent::{Agent, AgentBuilder, AgentCard},
    message::{Message, MessageOutput, Part, Role},
    runenv::{Sandbox, SandboxConfig, VolumeMount},
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiResult, AppError},
    model::{
        CreateSessionRequest, SendMessageRequest, SendMessageResponse, SessionListResponse,
        SessionResponse, UpdateSessionRequest,
    },
    repository::SessionAccess,
    state::AppState,
};

const DEFAULT_MODEL: &str = "openai/gpt-5.4-mini";

fn sandbox_name_for(id: &Uuid) -> String {
    let s = id.simple().to_string();
    format!("session-{}", &s[..12])
}

async fn build_sandbox(
    state: &Arc<AppState>,
    project_id: Uuid,
    session_id: Uuid,
) -> Result<Sandbox, String> {
    let uploads_host = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("uploads");
    tokio::fs::create_dir_all(&uploads_host)
        .await
        .map_err(|e| format!("failed to create uploads dir: {e}"))?;

    let volumes = vec![VolumeMount::Bind {
        host: uploads_host,
        guest: "/workspace/.uploads".to_string(),
        readonly: true,
    }];

    let sandbox_name = sandbox_name_for(&session_id);
    let cfg = SandboxConfig {
        name: Some(sandbox_name),
        persist: true,
        volumes,
        ..Default::default()
    };
    Sandbox::new(cfg).await.map_err(|e| e.to_string())
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
            "Your working directory is /workspace, which is read-write — you can freely ",
            "create, edit, and delete files there for intermediate work, scripts, and results. ",
            "Project files uploaded by the user are available read-only at /workspace/.uploads/. ",
            "Use `ls /workspace/.uploads` to see what files are available, and ",
            "`cat /workspace/.uploads/<path>` to read them. ",
            "To modify or analyse uploaded files, copy them into /workspace first. ",
            "New uploads appear in /workspace/.uploads immediately without restarting. ",
            "You MUST use the speedwagon tool to search the document corpus ",
            "before answering ANY factual question — even if you think you already know the answer. ",
            "Use bash and python tools for computation, data analysis, and code execution tasks. ",
            "Only skip tools for greetings or casual conversation.",
        ))
        .system_tools()
        .web_search_tool()
        .runenv(sandbox)
        .subagent(sw_spec)
        .build()
        .map_err(|e| e.to_string())
}

async fn resolve_agent_for(
    state: &Arc<AppState>,
    session_id: Uuid,
    project_id: Uuid,
) -> ApiResult<Arc<tokio::sync::Mutex<Agent>>> {
    if let Some(arc) = state.get_agent(&session_id) {
        return Ok(arc);
    }

    let history = state
        .repository
        .get_messages(session_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let sandbox = build_sandbox(state, project_id, session_id)
        .await
        .map_err(|e| AppError::internal(e))?;

    let mut agent = build_agent(sandbox)
        .await
        .map_err(|e| AppError::internal(e))?;

    agent.state.history = history;
    tracing::info!(%session_id, "agent lazy-created with history restored");

    if let Some(existing) = state.get_agent(&session_id) {
        return Ok(existing);
    }
    state.insert_agent(session_id, agent);
    Ok(state.get_agent(&session_id).unwrap())
}

// ── Session CRUD ──────────────────────────────────────────────────────────────

/// POST /projects/{project_id}/sessions
pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_id): Path<Uuid>,
    Json(_payload): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let is_member = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_member {
        return Err(AppError::forbidden("not a member of this project"));
    }

    let session = state
        .repository
        .create_session(project_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let sandbox_name = sandbox_name_for(&session.id);
    let sandbox = match build_sandbox(&state, project_id, session.id).await {
        Ok(s) => s,
        Err(e) => {
            let _ = state.repository.delete_session(session.id).await;
            return Err(AppError::internal(e));
        }
    };
    let agent = match build_agent(sandbox).await {
        Ok(a) => a,
        Err(e) => {
            let _ = ailoy::runenv::remove_persisted(&sandbox_name).await;
            let _ = state.repository.delete_session(session.id).await;
            return Err(AppError::internal(e));
        }
    };
    state.insert_agent(session.id, agent);

    tracing::info!(id = %session.id, sandbox = %sandbox_name, "session created");
    Ok((StatusCode::CREATED, Json(SessionResponse::from(session))))
}

/// GET /projects/{project_id}/sessions
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_id): Path<Uuid>,
) -> ApiResult<Json<SessionListResponse>> {
    let is_member = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_member {
        return Err(AppError::forbidden("not a member of this project"));
    }

    let sessions = state
        .repository
        .list_sessions_in_project(project_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(SessionListResponse {
        items: sessions.into_iter().map(SessionResponse::from).collect(),
    }))
}

/// GET /sessions/{session_id}
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<Uuid>,
) -> ApiResult<Json<SessionResponse>> {
    let (session, _access) = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    Ok(Json(SessionResponse::from(session)))
}

/// PATCH /sessions/{session_id} — share_mode change (creator or project owner)
pub async fn update_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<Uuid>,
    Json(payload): Json<UpdateSessionRequest>,
) -> ApiResult<Json<SessionResponse>> {
    let (session, access) = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    if !matches!(access, SessionAccess::Admin) {
        return Err(AppError::forbidden("only admins can change sharing"));
    }

    let updated = state
        .repository
        .update_session_share_mode(session.id, &payload.share_mode)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(SessionResponse::from(updated)))
}

pub(crate) async fn cleanup_session_resources(state: &Arc<AppState>, session_id: Uuid) {
    state.remove_agent(&session_id);
    let sandbox_name = sandbox_name_for(&session_id);
    if let Err(e) = ailoy::runenv::remove_persisted(&sandbox_name).await {
        tracing::warn!(%session_id, "failed to remove persisted sandbox: {e}");
    }
}

/// DELETE /sessions/{session_id} — creator or project owner
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (session, access) = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    if !matches!(access, SessionAccess::Admin) {
        return Err(AppError::forbidden("only admins can delete this session"));
    }

    state
        .repository
        .delete_session(session.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    cleanup_session_resources(&state, session_id).await;

    tracing::info!(%session_id, "session deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// POST /sessions/{session_id}/fork
pub async fn fork_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(source_session_id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let (source, _access) = state
        .repository
        .get_session_with_authz(source_session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    // Hold agent lock for entire fork — prevents send_message from running concurrently.
    // If agent is absent from cache, sandbox is already stopped.
    let _agent_guard = if let Some(arc) = state.get_agent(&source_session_id) {
        Some(
            arc.try_lock_owned()
                .map_err(|_| AppError::locked("session is currently in use"))?,
        )
    } else {
        None
    };

    let new_id = Uuid::new_v4();

    let new_session = state
        .repository
        .fork_session(source_session_id, new_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let source_cfg = SandboxConfig {
        name: Some(sandbox_name_for(&source_session_id)),
        persist: true,
        ..Default::default()
    };
    let source_sandbox = match Sandbox::new(source_cfg).await {
        Ok(s) => s,
        Err(e) => {
            let _ = state.repository.delete_session(new_id).await;
            return Err(AppError::internal(e.to_string()));
        }
    };

    let new_sandbox_name = sandbox_name_for(&new_id);
    let new_cfg = SandboxConfig {
        name: Some(new_sandbox_name.clone()),
        persist: true,
        ..Default::default()
    };

    match source_sandbox.fork(new_cfg).await {
        Ok(_) => {
            tracing::info!(
                source = %source_session_id,
                fork = %new_id,
                sandbox = %new_sandbox_name,
                project = %source.project_id,
                "session forked",
            );
            Ok((
                StatusCode::CREATED,
                Json(SessionResponse::from(new_session)),
            ))
        }
        Err(e) => {
            let _ = state.repository.delete_session(new_id).await;
            let _ = ailoy::runenv::remove_persisted(&new_sandbox_name).await;
            Err(AppError::internal(format!("sandbox fork failed: {e}")))
        }
    }
}

// ── Messages ──────────────────────────────────────────────────────────────────

/// GET /sessions/{session_id}/messages
pub async fn get_message_history(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<Uuid>,
) -> ApiResult<Json<Vec<Message>>> {
    let _ = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    let messages = state
        .repository
        .get_messages(session_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(messages))
}

/// DELETE /sessions/{session_id}/messages — creator or project owner
pub async fn clear_message_history(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let (session, access) = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    if !matches!(access, SessionAccess::Admin) {
        return Err(AppError::forbidden("only admins can clear history"));
    }

    // Acquire agent lock before clearing so concurrent sends can't re-persist old messages.
    if let Some(arc) = state.get_agent(&session_id) {
        let mut agent = arc.lock().await;
        state
            .repository
            .clear_messages(session.id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        agent.state.history.clear();
    } else {
        state
            .repository
            .clear_messages(session.id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    }

    tracing::info!(%session_id, "message history cleared");
    Ok(StatusCode::NO_CONTENT)
}

/// POST /sessions/{session_id}/messages
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<Json<SendMessageResponse>> {
    let (session, access) = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    if matches!(access, SessionAccess::ReadOnlyMember) {
        return Err(AppError::forbidden("read-only access to this session"));
    }

    let agent_arc = resolve_agent_for(&state, session.id, session.project_id).await?;

    let mut agent = agent_arc
        .try_lock()
        .map_err(|_| AppError::locked("session is currently in use"))?;

    let prev_len = agent.get_history().len();
    let msg = Message::new(Role::User).with_contents([Part::text(payload.content)]);
    let mut run = agent.run(msg);
    let mut outputs: Vec<MessageOutput> = Vec::new();
    while let Some(item) = run.next().await {
        outputs.push(item.map_err(|e| AppError::internal(e.to_string()))?);
    }
    drop(run);
    let new_messages = agent.get_history()[prev_len..].to_vec();
    drop(agent);

    state
        .repository
        .append_messages(session_id, &new_messages)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(outputs))
}

/// POST /sessions/{session_id}/messages/stream
pub async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<
    NoApi<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static>>,
> {
    let (session, access) = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    if matches!(access, SessionAccess::ReadOnlyMember) {
        return Err(AppError::forbidden("read-only access to this session"));
    }

    let agent_arc = resolve_agent_for(&state, session.id, session.project_id).await?;

    // Acquire OwnedMutexGuard — held for entire SSE stream lifetime.
    // Returns 423 immediately if another request holds the lock.
    let guard = agent_arc
        .clone()
        .try_lock_owned()
        .map_err(|_| AppError::locked("session is currently in use"))?;

    let prev_len = guard.get_history().len();
    let repo = state.repository.clone();
    let content = payload.content;

    let stream = async_stream::stream! {
        let mut agent = guard;  // OwnedMutexGuard moved in — lock held for stream lifetime
        let msg = Message::new(Role::User).with_contents([Part::text(content)]);
        let mut run = agent.run(msg);

        let mut run_error: Option<String> = None;
        while let Some(item) = run.next().await {
            match item {
                Ok(output) => {
                    let json = serde_json::to_string(&output)
                        .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
                    yield Ok::<Event, Infallible>(
                        Event::default().event("message").data(json),
                    );
                }
                Err(e) => {
                    run_error = Some(e.to_string());
                    break;  // Must break before accessing `agent` — `run` borrows it
                }
            }
        }
        drop(run);

        if let Some(err) = run_error {
            // Truncate in-memory history to match DB state so the agent stays consistent.
            agent.state.history.truncate(prev_len);
            drop(agent);
            yield Ok(Event::default().event("error").data(err));
            return;
        }

        let new_msgs = agent.get_history()[prev_len..].to_vec();
        drop(agent);  // Release OwnedMutexGuard

        if let Err(e) = repo.append_messages(session_id, &new_msgs).await {
            tracing::error!(%session_id, "failed to persist messages: {e}");
        }

        yield Ok(Event::default().event("done").data("[DONE]"));
    };

    Ok(NoApi(Sse::new(stream).keep_alive(KeepAlive::default())))
}
