use std::{convert::Infallible, sync::Arc};

use aide::NoApi;
use ailoy::{
    agent::{Agent, AgentBuilder, AgentCard, AgentSpec},
    message::{Message, MessageOutput, Part, Role},
    runenv::{RunEnv, Sandbox, SandboxConfig, VolumeMount},
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
        CreateSessionRequest, MessageSender, SendMessageRequest, SendMessageResponse,
        SessionListResponse, SessionMessageListResponse, SessionMessageResponse, SessionResponse,
        UpdateSessionRequest,
    },
    repository::{DbSenderKind, NewSessionMessage, SessionAccess},
    services::session_title::generate_session_title,
    state::AppState,
};

const DEFAULT_MODEL: &str = "anthropic/claude-haiku-4-5";
const TOP_LEVEL_AGENT_NAME: &str = "agent-k";

pub(crate) fn sandbox_name_for(id: &Uuid) -> String {
    let s = id.simple().to_string();
    format!("session-{}", &s[..12])
}

pub(crate) async fn build_sandbox(
    state: &Arc<AppState>,
    project_id: Uuid,
    session_id: Uuid,
) -> Result<RunEnv, String> {
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
    RunEnv::sandbox(cfg).await.map_err(|e| e.to_string())
}

pub(crate) async fn build_agent(runenv: RunEnv) -> Result<Agent, String> {
    // TODO: Remove the subagent. This is added for testing frontend UI.
    let math_spec = AgentSpec::new(DEFAULT_MODEL)
        .instruction(
            "You are a math expert. Solve any mathematical problem step by step, \
            showing your reasoning clearly. Return the final answer at the end.",
        )
        .card(AgentCard {
            name: "math".into(),
            description: "Solve mathematical problems step by step. \
                Use for arithmetic, algebra, calculus, statistics, or any numeric reasoning."
                .into(),
            skills: vec![],
        });

    AgentBuilder::new(DEFAULT_MODEL)
        .instruction(concat!(
            "You are a versatile assistant with access to code execution tools ",
            "(bash, python), web search, and a math subagent. ",
            "Your working directory is /workspace, which is read-write — you can freely ",
            "create, edit, and delete files there for intermediate work, scripts, and results. ",
            "Project files uploaded by the user are available read-only at /workspace/.uploads/. ",
            "Use `ls /workspace/.uploads` to see what files are available, and ",
            "`cat /workspace/.uploads/<path>` to read them. ",
            "To modify or analyse uploaded files, copy them into /workspace first. ",
            "New uploads appear in /workspace/.uploads immediately without restarting. ",
            "You MUST delegate ALL math and numeric problems to the math tool. ",
            "Use bash and python tools for computation, data analysis, and code execution tasks. ",
            "Only skip tools for greetings or casual conversation.",
        ))
        .system_tools()
        .web_search_tool(vec![])
        .runenv(runenv)
        .subagent(math_spec)
        .build()
        .map_err(|e| e.to_string())
}

/// Attribute each persisted message to a sender using `MessageOutput` metadata.
///
/// `agent.get_history()[prev_len..]` always starts with the user query (pushed
/// by `Agent::run` before any outputs are emitted), followed by one entry per
/// depth-0 `MessageOutput` in emission order.  This function mirrors that layout:
/// the first sender is always the user; subsequent senders are derived from the
/// depth-0 outputs via `source_agent` (set by ailoy's `stamp_source_agent`).
///
/// Depth ≥ 1 outputs are skipped because ailoy does not push them into history.
/// Pair each message in `messages` with its sender attribution derived from
/// the agent run's `outputs`. The first message is the user query;
/// subsequent agent messages are tagged with `source_agent` when present.
pub(crate) fn attribute_messages(
    messages: Vec<Message>,
    outputs: &[MessageOutput],
    user_id: Uuid,
) -> Vec<NewSessionMessage> {
    let senders = classify_senders_from_outputs(outputs, user_id);
    messages
        .into_iter()
        .zip(senders)
        .map(
            |(message, (sender_kind, sender_name, sender_user_id))| NewSessionMessage {
                message,
                sender_kind,
                sender_name,
                sender_user_id,
            },
        )
        .collect()
}

fn classify_senders_from_outputs(
    outputs: &[MessageOutput],
    user_id: Uuid,
) -> Vec<(DbSenderKind, Option<String>, Option<Uuid>)> {
    let mut senders = vec![(DbSenderKind::User, None, Some(user_id))];

    for output in outputs {
        if !matches!(output.depth, None | Some(0)) {
            continue;
        }
        match output.message.role {
            Role::User => continue,
            _ => {
                let name = output
                    .source_agent
                    .clone()
                    .unwrap_or_else(|| TOP_LEVEL_AGENT_NAME.to_string());
                senders.push((DbSenderKind::Agent, Some(name), None));
            }
        }
    }

    senders
}

async fn resolve_agent_for(
    state: &Arc<AppState>,
    session_id: Uuid,
    project_id: Uuid,
) -> ApiResult<Arc<tokio::sync::Mutex<Agent>>> {
    if let Some(arc) = state.get_agent(&session_id) {
        return Ok(arc);
    }

    let rows = state
        .repository
        .get_messages(session_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let history: Vec<Message> = rows.into_iter().map(|r| r.message).collect();

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

/// POST /sessions
/// body must include `project_id`; user must be a member of that project.
pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Json(payload): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let project_id = payload.project_id;
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
    Ok((
        StatusCode::CREATED,
        Json(SessionResponse::from_db(session, 0)),
    ))
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ListSessionsQuery {
    pub project_id: Option<Uuid>,
}

/// GET /sessions?project_id=...
/// `project_id` is optional — omit to list all sessions across projects the user can access.
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    axum::extract::Query(q): axum::extract::Query<ListSessionsQuery>,
) -> ApiResult<Json<SessionListResponse>> {
    let sessions = match q.project_id {
        Some(project_id) => {
            let is_member = state
                .repository
                .user_in_project(auth_user.id, project_id)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?;
            if !is_member {
                return Err(AppError::forbidden("not a member of this project"));
            }
            state
                .repository
                .list_sessions_in_project(project_id, auth_user.id)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?
        }
        None => state
            .repository
            .list_sessions_for_user(auth_user.id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    };

    let session_ids: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();
    let unread_map = state
        .repository
        .count_unread_batch_for_user(&session_ids, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let items: Vec<SessionResponse> = sessions
        .into_iter()
        .map(|s| {
            let unread = unread_map.get(&s.id).copied().unwrap_or(0);
            SessionResponse::from_db(s, unread)
        })
        .collect();

    Ok(Json(SessionListResponse { items }))
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

    let unread = state
        .repository
        .count_session_unread(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(SessionResponse::from_db(session, unread)))
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

    let unread = state
        .repository
        .count_session_unread(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(SessionResponse::from_db(updated, unread)))
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

    // Mark the forked session as fully read for the creator
    let _ = state
        .repository
        .mark_session_read(new_id, auth_user.id)
        .await;

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
                Json(SessionResponse::from_db(new_session, 0)),
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
) -> ApiResult<Json<SessionMessageListResponse>> {
    let (_session, _access) = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    let rows = state
        .repository
        .get_messages(session_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let items = rows
        .into_iter()
        .map(|r| -> ApiResult<SessionMessageResponse> {
            let sender = match r.sender_kind {
                DbSenderKind::User => MessageSender::User {
                    user_id: r
                        .sender_user_id
                        .ok_or_else(|| AppError::internal("user message missing sender_user_id"))?,
                },
                DbSenderKind::Agent => MessageSender::Agent {
                    name: r
                        .sender_name
                        .unwrap_or_else(|| TOP_LEVEL_AGENT_NAME.to_string()),
                },
            };
            Ok(SessionMessageResponse {
                message: r.message,
                sender,
                created_at: r.created_at,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Auto-mark all messages as read for this user
    let _ = state
        .repository
        .mark_session_read(session_id, auth_user.id)
        .await;

    Ok(Json(SessionMessageListResponse { items }))
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

    let need_title = session.title.is_none();
    let project_id = session.project_id;

    // Spawn title generation immediately — runs concurrently with the agent run
    if need_title {
        let repo_title = state.repository.clone();
        let ws_tx = state.ws_tx.clone();
        let first_msg = payload.content.clone();
        tokio::spawn(async move {
            let title = generate_session_title(&first_msg).await;
            if repo_title
                .set_session_title(session_id, &title)
                .await
                .is_ok()
            {
                let _ = ws_tx.send(crate::events::WsEvent::SessionTitleUpdated {
                    session_id: session_id.to_string(),
                    project_id: project_id.to_string(),
                    title,
                });
            }
        });
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

    let to_persist = attribute_messages(new_messages, &outputs, auth_user.id);

    state
        .repository
        .append_messages(session_id, &to_persist)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let _ = state
        .repository
        .mark_session_read(session_id, auth_user.id)
        .await;

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
    let need_title = session.title.is_none();
    let sender_id = auth_user.id;
    let project_id = session.project_id;

    // Spawn title generation immediately — runs concurrently with the agent stream
    if need_title {
        let repo_title = repo.clone();
        let ws_tx = state.ws_tx.clone();
        let first_msg = content.clone();
        tokio::spawn(async move {
            tracing::info!("starting title generation");
            let title = generate_session_title(&first_msg).await;
            tracing::info!("title generation finished");
            if repo_title
                .set_session_title(session_id, &title)
                .await
                .is_ok()
            {
                tracing::info!("send title via websocket");
                let _ = ws_tx.send(crate::events::WsEvent::SessionTitleUpdated {
                    session_id: session_id.to_string(),
                    project_id: project_id.to_string(),
                    title,
                });
            }
        });
    }

    let stream = async_stream::stream! {
        let mut agent = guard;  // OwnedMutexGuard moved in — lock held for stream lifetime
        let msg = Message::new(Role::User).with_contents([Part::text(content)]);
        let mut run = agent.run(msg);

        let mut run_error: Option<String> = None;
        let mut depth0_outputs: Vec<MessageOutput> = Vec::new();
        while let Some(item) = run.next().await {
            match item {
                Ok(output) => {
                    if matches!(output.depth, None | Some(0)) {
                        depth0_outputs.push(output.clone());
                    }
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

        let to_persist = attribute_messages(new_msgs, &depth0_outputs, sender_id);

        if let Err(e) = repo.append_messages(session_id, &to_persist).await {
            tracing::error!(%session_id, "failed to persist messages: {e}");
        }

        // Auto-mark sender as having read
        let _ = repo.mark_session_read(session_id, sender_id).await;

        tracing::info!("message stream finished");
        yield Ok(Event::default().event("done").data("[DONE]"));
    };

    Ok(NoApi(Sse::new(stream).keep_alive(KeepAlive::default())))
}
