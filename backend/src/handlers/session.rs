use std::{collections::HashSet, sync::Arc};

use agent_k::agents::{
    GUEST_ATTACHED_DIR, GUEST_SHARED_DIR, get_coworker_agent_runenv, get_coworker_agent_spec,
    get_deep_research_agent_runenv, get_deep_research_agent_spec,
};
use ailoy::{
    agent::{Agent, AgentState},
    message::{Message, MessageOutput, Part, Role},
    runenv::{Sandbox, SharedMachine},
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiResult, AppError},
    events::{RunUserMessage, WsEvent},
    handlers::dirent::{
        DirentScope, copy_dir_recursive, enforce_scope_access, parse_dirent_path, scope_root,
    },
    model::{
        CreateSessionRequest, MessageSender, RunAck, SendMessageRequest, SessionListResponse,
        SessionMessageListResponse, SessionMessageResponse, SessionResponse, UpdateSessionRequest,
    },
    repository::{DbSenderKind, NewSessionMessage, PrefixLookup, SessionAccess, ShareMode},
    services::session_title::generate_session_title,
    state::AppState,
};

const TOP_LEVEL_AGENT_NAME: &str = "agent-k";
const SANDBOX_IMAGE: &str = "brekkylab/agent-k:latest";

pub(crate) fn sandbox_name_for(id: &Uuid) -> String {
    let s = id.simple().to_string();
    format!("session-{}", &s[..12])
}

fn session_root(state: &AppState, project_id: Uuid, session_id: Uuid) -> std::path::PathBuf {
    state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("sessions")
        .join(session_id.to_string())
}

fn session_dirs(
    state: &AppState,
    project_id: Uuid,
    session_id: Uuid,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let root = session_root(state, project_id, session_id);
    let shared = state
        .data_root
        .join("projects")
        .join(project_id.to_string())
        .join("shared");
    (root.join("inputs"), shared, root.join("artifacts"))
}

pub async fn build_session_agent(
    state: &Arc<AppState>,
    project_id: Uuid,
    session_id: Uuid,
) -> Result<Agent, String> {
    let (inputs, shared, artifacts) = session_dirs(state, project_id, session_id);
    for d in [&inputs, &shared, &artifacts] {
        tokio::fs::create_dir_all(d)
            .await
            .map_err(|e| format!("failed to create dir {}: {e}", d.display()))?;
    }
    // Resolve the effective model from the session's agent_type + optional pin.
    let (agent_type, model_pin) = match state.repository.get_session(session_id).await {
        Ok(Some(s)) => (s.agent_type, s.model),
        Ok(None) => (None, None),
        Err(e) => return Err(e.to_string()),
    };
    // The same agent type drives both model resolution and dispatch; unknown → coworker.
    let agent_type = agent_type
        .as_deref()
        .and_then(crate::model::AgentType::from_str)
        .unwrap_or(crate::model::AgentType::Coworker);
    // Resolve within the project's chain for this agent (built-in default if uncustomized).
    let project_chains = match state.repository.get_project(project_id).await {
        Ok(Some(p)) => crate::model::ProjectChains::parse(p.recommended_chains.as_deref()),
        _ => crate::model::ProjectChains::default(),
    };
    let chain = project_chains.chain_for(agent_type);
    let model = crate::model::resolve_model_in(&chain, model_pin.as_deref());

    use crate::model::AgentType;
    let agent = match agent_type {
        AgentType::DeepResearch => {
            let spec = get_deep_research_agent_spec(TOP_LEVEL_AGENT_NAME, &model);
            let runenv = get_deep_research_agent_runenv(&artifacts)
                .await
                .map_err(|e| e.to_string())?;
            let state = AgentState::new().with_runenv(SharedMachine::new(runenv));
            Agent::try_with_state(spec, state).map_err(|e| e.to_string())?
        }
        AgentType::Buddy => agent_k::agents::get_buddy_agent(TOP_LEVEL_AGENT_NAME, &model)
            .map_err(|e| e.to_string())?,
        // Speedwagon: Q&A over the global document corpus (not the session
        // sandbox); non-sandboxed. Interim until it moves to a sandbox agent.
        AgentType::Speedwagon => {
            let spec = agent_k::agents::SpeedwagonSpec::new()
                .model(&model)
                .into_spec();
            Agent::try_new(spec).map_err(|e| e.to_string())?
        }
        // Coworker runs the sandboxed coworker agent over the session's files.
        AgentType::Coworker => {
            let spec = get_coworker_agent_spec(TOP_LEVEL_AGENT_NAME, &model, true);
            let runenv = get_coworker_agent_runenv(&inputs, &shared, &artifacts)
                .await
                .map_err(|e| e.to_string())?;
            let state = AgentState::new().with_runenv(SharedMachine::new(runenv));
            Agent::try_with_state(spec, state).map_err(|e| e.to_string())?
        }
    };

    // Pin the resolved model onto a "recommended" (NULL) session so it stays
    // stable if the chain/availability later changes. Best-effort; pins untouched.
    if model_pin.is_none()
        && let Err(e) = state.repository.set_session_model(session_id, &model).await
    {
        tracing::warn!(session = %session_id, "failed to persist resolved model: {e}");
    }

    Ok(agent)
}

/// Builds the LLM hint note for attached files, using the correct sandbox path per scope.
///
/// - `inputs/` is bind-mounted flat at `GUEST_ATTACHED_DIR`, so only the basename is needed.
/// - `shared/` is bind-mounted at `GUEST_SHARED_DIR` preserving directory structure, so the
///   full tail path (relative to the shared root) is needed.
pub fn build_attachment_note(attachments: &[String]) -> String {
    let hints = attachments
        .iter()
        .filter_map(|path| {
            parse_dirent_path(path).ok().map(|parsed| {
                let tail = parsed.tail.to_string_lossy();
                match &parsed.scope {
                    DirentScope::Inputs { .. } => {
                        let name = parsed
                            .tail
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| tail.into_owned());
                        format!("{GUEST_ATTACHED_DIR}/{name}")
                    }
                    DirentScope::Shared { .. } | DirentScope::Artifacts { .. } => {
                        format!("{GUEST_SHARED_DIR}/{tail}")
                    }
                }
            })
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[Attached files: {hints}]")
}

/// Re-injects the attachment note into a stored user message when reconstructing LLM history.
///
/// Messages are persisted without the note so the frontend can display clean content.
/// This restores the note for the agent's context on the next run.
pub fn inject_attachment_note(mut msg: Message, attachments: &[String]) -> Message {
    if attachments.is_empty() {
        return msg;
    }
    let current_text = msg
        .contents
        .iter()
        .filter_map(|p| {
            if let Part::Text { text } = p {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");
    let note = build_attachment_note(attachments);
    msg.contents = vec![Part::text(format!("{current_text}\n\n{note}"))];
    msg
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
/// Walk `artifacts_dir` recursively and return the set of scope-relative file paths.
/// Returns an empty set if the directory does not exist.
async fn collect_artifact_paths(artifacts_dir: &std::path::Path) -> HashSet<String> {
    let mut paths = HashSet::new();
    let mut stack = vec![artifacts_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        loop {
            match rd.next_entry().await {
                Ok(Some(entry)) => match entry.metadata().await {
                    Ok(m) if m.is_dir() => stack.push(entry.path()),
                    Ok(_) => {
                        if let Ok(rel) = entry.path().strip_prefix(artifacts_dir) {
                            paths.insert(rel.to_string_lossy().into_owned());
                        }
                    }
                    Err(_) => {}
                },
                Ok(None) => break,
                Err(_) => break,
            }
        }
    }
    paths
}

pub(crate) fn attribute_messages(
    messages: Vec<Message>,
    outputs: &[MessageOutput],
    user_id: Uuid,
    user_attachments: Vec<String>,
) -> Vec<NewSessionMessage> {
    let senders = classify_senders_from_outputs(outputs, user_id);
    messages
        .into_iter()
        .zip(senders)
        .enumerate()
        .map(
            |(i, (message, (sender_kind, sender_name, sender_user_id)))| NewSessionMessage {
                message,
                sender_kind,
                sender_name,
                sender_user_id,
                attachments: if i == 0 {
                    user_attachments.clone()
                } else {
                    vec![]
                },
                artifacts: vec![],
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
    // Messages are stored without the attachment note; re-inject it so the agent sees
    // the correct file hints when replaying history.
    let history: Vec<Message> = rows
        .into_iter()
        .map(|r| inject_attachment_note(r.message, &r.attachments))
        .collect();

    let mut agent = build_session_agent(state, project_id, session_id)
        .await
        .map_err(|e| {
            tracing::error!(%session_id, %project_id, "build_session_agent failed: {e}");
            AppError::internal(e)
        })?;

    agent.state.history.extend(history);
    tracing::info!(%session_id, "agent lazy-created with history restored");

    if let Some(existing) = state.get_agent(&session_id) {
        return Ok(existing);
    }
    state.insert_agent(session_id, agent);
    Ok(state.get_agent(&session_id).unwrap())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Resolve a path param to a session UUID.
///
/// Accepts both a full UUID string and a prefix (any length, hex-only or with
/// hyphens). Returns 409 when the prefix matches multiple sessions and 404
/// when nothing matches.
async fn resolve_session_id(state: &Arc<AppState>, session_ref: &str) -> ApiResult<Uuid> {
    if let Ok(uuid) = Uuid::parse_str(session_ref) {
        return Ok(uuid);
    }
    match state
        .repository
        .lookup_session_by_prefix(session_ref)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        PrefixLookup::Unique(id) => Ok(id),
        PrefixLookup::Ambiguous(_) => Err(AppError::conflict(
            "ambiguous session prefix; provide more characters",
        )),
        PrefixLookup::None => Err(AppError::not_found("session not found")),
    }
}

async fn validate_attachments(
    state: &Arc<AppState>,
    auth_user: &AuthUser,
    session_id: Uuid,
    project_id: Uuid,
    attachments: &[String],
) -> ApiResult<()> {
    for path in attachments {
        let parsed = parse_dirent_path(path)
            .map_err(|_| AppError::bad_request(format!("invalid attachment path: {path}")))?;
        match &parsed.scope {
            DirentScope::Inputs {
                session_id: sid, ..
            } if *sid == session_id => {}
            DirentScope::Shared { project_id: pid } if *pid == project_id => {}
            _ => {
                return Err(AppError::bad_request(format!(
                    "attachment path not valid for this session: {path}"
                )));
            }
        }
        enforce_scope_access(state, auth_user, &parsed.scope, false).await?;
        let host_path = scope_root(state, &parsed.scope).join(&parsed.tail);
        let meta = tokio::fs::symlink_metadata(&host_path)
            .await
            .map_err(|_| AppError::not_found(format!("attachment not found: {path}")))?;
        if meta.is_symlink() {
            return Err(AppError::bad_request(format!(
                "attachment path must not be a symlink: {path}"
            )));
        }
        if !meta.is_file() {
            return Err(AppError::bad_request(format!(
                "attachment must be a file: {path}"
            )));
        }
    }
    Ok(())
}

// ── Session CRUD ──────────────────────────────────────────────────────────────

/// POST /sessions
/// body must include `project_ref` (UUID or slug); user must be a member of that project.
pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Json(payload): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let project_id = super::project::resolve_project_id(&state, &payload.project_ref).await?;
    let is_member = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_member {
        return Err(AppError::forbidden("not a member of this project"));
    }

    // Validate agent_type/model against the catalog (unknown = client error).
    // Provider availability is not required — an unavailable pin falls through.
    let agent_type = match payload.agent_type.as_deref() {
        None => None,
        Some(s) => Some(
            crate::model::AgentType::from_str(s)
                .ok_or_else(|| AppError::bad_request(format!("unknown agent_type: {s}")))?
                .as_str()
                .to_string(),
        ),
    };
    if let Some(model) = payload.model.as_deref() {
        if crate::model::catalog_entry(model).is_none() {
            return Err(AppError::bad_request(format!("unknown model: {model}")));
        }
    }

    let session = state
        .repository
        .create_session_full(
            project_id,
            auth_user.id,
            crate::repository::SessionOrigin::User,
            agent_type.as_deref(),
            payload.model.as_deref(),
        )
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    tracing::info!(id = %session.id, agent_type = ?session.agent_type, model = ?session.model, "session created");
    Ok((
        StatusCode::CREATED,
        Json(SessionResponse::from_db(session, 0)),
    ))
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ListSessionsQuery {
    /// Project UUID, active slug, or retired slug — backend resolves all three.
    pub project_ref: Option<String>,
}

/// GET /sessions?project_ref=...
/// `project_ref` is optional — omit to list all sessions across projects the user can access.
pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    axum::extract::Query(q): axum::extract::Query<ListSessionsQuery>,
) -> ApiResult<Json<SessionListResponse>> {
    let sessions = match q.project_ref {
        Some(project_ref) => {
            let project_id = super::project::resolve_project_id(&state, &project_ref).await?;
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
    Path(session_ref): Path<String>,
) -> ApiResult<Json<SessionResponse>> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
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
    Path(session_ref): Path<String>,
    Json(payload): Json<UpdateSessionRequest>,
) -> ApiResult<Json<SessionResponse>> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
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

    // [R2-4] share_mode downgrade to private: evict non-creator subscribers immediately
    // instead of waiting for Task D's 60-second poll.
    if payload.share_mode == ShareMode::Private && session.share_mode != ShareMode::Private {
        if let Ok(members) = state
            .repository
            .list_project_members(session.project_id)
            .await
        {
            for (member, _) in members {
                if member.id == session.creator_id {
                    continue;
                }
                let _ = state.ws_tx.send(WsEvent::AccessRevoked {
                    session_id: session_id.to_string(),
                    user_id: member.id.to_string(),
                });
            }
        }
    }

    let unread = state
        .repository
        .count_session_unread(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(SessionResponse::from_db(updated, unread)))
}

/// DELETE /sessions/{session_id} — creator or project owner
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_ref): Path<String>,
) -> ApiResult<StatusCode> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
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

    if let Err(e) = state
        .storage
        .remove_session(session.project_id.to_string(), session_id.to_string())
        .await
    {
        tracing::warn!(%session_id, "failed to remove session dir: {e}");
    }

    tracing::info!(%session_id, "session deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// POST /sessions/{session_id}/fork
pub async fn fork_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(source_session_ref): Path<String>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    todo!()
    // let source_session_id = resolve_session_id(&state, &source_session_ref).await?;
    // let (source, _access) = state
    //     .repository
    //     .get_session_with_authz(source_session_id, auth_user.id)
    //     .await
    //     .map_err(|e| AppError::internal(e.to_string()))?
    //     .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    // // Hold agent lock for entire fork — prevents send_message from running concurrently.
    // // If agent is absent from cache, sandbox is already stopped.
    // let _agent_guard = if let Some(arc) = state.get_agent(&source_session_id) {
    //     Some(
    //         arc.try_lock_owned()
    //             .map_err(|_| AppError::locked("session is currently in use"))?,
    //     )
    // } else {
    //     None
    // };

    // let new_id = Uuid::new_v4();

    // let new_session = state
    //     .repository
    //     .fork_session(source_session_id, new_id, auth_user.id)
    //     .await
    //     .map_err(|e| AppError::internal(e.to_string()))?;

    // // Mark the forked session as fully read for the creator
    // let _ = state
    //     .repository
    //     .mark_session_read(new_id, auth_user.id)
    //     .await;

    // // Mirror the source session's host files into the fork so the snapshot matches
    // // the VM upper.ext4 clone. shared/ is project-level and intentionally excluded.
    // let (src_inputs, _, src_artifacts) = session_dirs(&state, source.project_id, source_session_id);
    // let (new_inputs, _, new_artifacts) = session_dirs(&state, source.project_id, new_id);
    // for (src, dst) in [(&src_inputs, &new_inputs), (&src_artifacts, &new_artifacts)] {
    //     let res = if tokio::fs::try_exists(src).await.unwrap_or(false) {
    //         copy_dir_recursive(src, dst).await
    //     } else {
    //         tokio::fs::create_dir_all(dst).await
    //     };
    //     if let Err(e) = res {
    //         tracing::warn!(
    //             source = %source_session_id,
    //             fork = %new_id,
    //             dir = ?dst,
    //             "failed to copy session dir on fork: {e}",
    //         );
    //     }
    // }

    // let source_cfg = SandboxConfig {
    //     name: Some(sandbox_name_for(&source_session_id)),
    //     image: SANDBOX_IMAGE.into(),
    //     persist: true,
    //     ..Default::default()
    // };
    // let source_sandbox = match Sandbox::new(source_cfg).await {
    //     Ok(s) => s,
    //     Err(e) => {
    //         let _ = state.repository.delete_session(new_id).await;
    //         // Clean up pre-created session dirs
    //         let session_rt = session_root(&state, source.project_id, new_id);
    //         let _ = tokio::fs::remove_dir_all(&session_rt).await;
    //         return Err(AppError::internal(e.to_string()));
    //     }
    // };

    // let new_sandbox_name = sandbox_name_for(&new_id);
    // let new_cfg = SandboxConfig {
    //     name: Some(new_sandbox_name.clone()),
    //     image: SANDBOX_IMAGE.into(),
    //     persist: true,
    //     ..Default::default()
    // };

    // match source_sandbox.fork(new_cfg).await {
    //     Ok(_) => {
    //         tracing::info!(
    //             source = %source_session_id,
    //             fork = %new_id,
    //             sandbox = %new_sandbox_name,
    //             project = %source.project_id,
    //             "session forked",
    //         );
    //         Ok((
    //             StatusCode::CREATED,
    //             Json(SessionResponse::from_db(new_session, 0)),
    //         ))
    //     }
    //     Err(e) => {
    //         let _ = state.repository.delete_session(new_id).await;
    //         let _ = Sandbox::remove_persisted(&new_sandbox_name).await;
    //         let session_rt = session_root(&state, source.project_id, new_id);
    //         let _ = tokio::fs::remove_dir_all(&session_rt).await;
    //         Err(AppError::internal(format!("sandbox fork failed: {e}")))
    //     }
    // }
}

// ── Messages ──────────────────────────────────────────────────────────────────

/// GET /sessions/{session_id}/messages
pub async fn get_message_history(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_ref): Path<String>,
) -> ApiResult<Json<SessionMessageListResponse>> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
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
                attachments: r.attachments,
                artifacts: r.artifacts,
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
    Path(session_ref): Path<String>,
) -> ApiResult<StatusCode> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
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
        // Keep the system message (instruction + Available Skills) so the
        // agent retains its identity after the conversation is wiped.
        agent.state.history.retain(|m| m.role == Role::System);
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
///
/// Fire-and-forget trigger: validates the request synchronously, registers an
/// active run, broadcasts `AgentRunStarted`, then spawns the agent run on a
/// background task and returns `202`-style `RunAck` immediately. All subsequent
/// progress is delivered over the WebSocket channel (`AgentMessage`,
/// `AgentError`, `AgentRunDone`).
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_ref): Path<String>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<(StatusCode, Json<RunAck>)> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
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

    // Validate attachments
    let attachments = payload.attachments.clone().unwrap_or_default();
    validate_attachments(
        &state,
        &auth_user,
        session_id,
        session.project_id,
        &attachments,
    )
    .await?;

    let raw_content = payload.content.clone();
    let content_with_note = if attachments.is_empty() {
        raw_content.clone()
    } else {
        let note = build_attachment_note(&attachments);
        format!("{raw_content}\n\n{note}")
    };

    let agent_arc = resolve_agent_for(&state, session.id, session.project_id).await?;

    // Acquire OwnedMutexGuard — moved into the background task and held for the
    // run's lifetime. Returns 423 immediately if another request holds the lock.
    let guard = agent_arc
        .clone()
        .try_lock_owned()
        .map_err(|_| AppError::locked("session is currently in use"))?;

    let prev_len = guard.get_history().len();
    let sender_id = auth_user.id;
    let (_, _, artifacts_dir) = session_dirs(&state, project_id, session_id);

    // Register the active run and broadcast its start before the agent begins.
    let now = chrono::Utc::now().to_rfc3339();
    let user_message = RunUserMessage {
        sender_user_id: sender_id.to_string(),
        content: raw_content.clone(),
        attachments: attachments.clone(),
        created_at: now,
    };
    let run_id = state.start_run(session_id, user_message.clone());
    let _ = state.ws_tx.send(WsEvent::AgentRunStarted {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        user_message,
    });

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
                let _ = ws_tx.send(WsEvent::SessionTitleUpdated {
                    session_id: session_id.to_string(),
                    project_id: project_id.to_string(),
                    title,
                });
            }
        });
    }

    // Run the agent on a background task — progress is streamed over the WebSocket.
    let state2 = state.clone();
    let run_handle = tokio::spawn(async move {
        let mut agent = guard; // OwnedMutexGuard — held for the run's lifetime
        let prev_artifacts = collect_artifact_paths(&artifacts_dir).await;
        let msg = Message::new(Role::User).with_contents([Part::text(content_with_note)]);
        let mut run = agent.run(msg);

        let mut run_error: Option<String> = None;
        let mut depth0_outputs: Vec<MessageOutput> = Vec::new();
        while let Some(item) = run.next().await {
            match item {
                Ok(output) => {
                    if matches!(output.depth, None | Some(0)) {
                        depth0_outputs.push(output.clone());
                    }
                    if let Some(seq) = state2.push_output(&session_id, output.clone()).await {
                        let _ = state2.ws_tx.send(WsEvent::AgentMessage {
                            session_id: session_id.to_string(),
                            run_id: run_id.to_string(),
                            seq,
                            output,
                        });
                    }
                }
                Err(e) => {
                    run_error = Some(e.to_string());
                    break; // Must break before accessing `agent` — `run` borrows it
                }
            }
        }
        drop(run);

        if let Some(err) = run_error {
            tracing::error!(%session_id, "agent run failed: {err}");
            // Truncate in-memory history to match DB state so the agent stays consistent.
            agent.state.history.truncate(prev_len);
            drop(agent);
            state2.end_run(&session_id);
            let _ = state2.ws_tx.send(WsEvent::AgentError {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                message: err,
            });
            return;
        }

        let mut new_msgs = agent.get_history()[prev_len..].to_vec();

        // Strip the attachment note before persisting — clean content is stored in DB and the
        // note is reconstructed from the `attachments` column when history is replayed.
        if !attachments.is_empty() {
            if let Some(first) = new_msgs.first_mut() {
                first.contents = vec![Part::text(raw_content.clone())];
            }
        }

        let current_artifacts = collect_artifact_paths(&artifacts_dir).await;
        let new_artifacts: Vec<String> = current_artifacts
            .difference(&prev_artifacts)
            .cloned()
            .collect();

        let mut to_persist =
            attribute_messages(new_msgs, &depth0_outputs, sender_id, attachments.clone());
        if !new_artifacts.is_empty() {
            if let Some(last_agent) = to_persist
                .iter_mut()
                .rev()
                .find(|m| matches!(m.sender_kind, crate::repository::DbSenderKind::Agent))
            {
                last_agent.artifacts = new_artifacts;
            }
        }

        // [A] Persist before releasing the agent lock. On failure: roll back in-memory history,
        // send AgentError (not AgentRunDone), and return — the DB remains consistent.
        if let Err(e) = state2
            .repository
            .append_messages(session_id, &to_persist)
            .await
        {
            tracing::error!(%session_id, "failed to persist messages: {e}");
            agent.state.history.truncate(prev_len);
            drop(agent);
            state2.end_run(&session_id);
            let _ = state2.ws_tx.send(WsEvent::AgentError {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                message: "응답 저장에 실패했습니다. 다시 시도해주세요.".to_string(),
            });
            return;
        }
        drop(agent); // Release OwnedMutexGuard only after successful persist

        // Intentionally NOT calling mark_session_read here: the sender should
        // only be considered to have read the agent's reply when they actually
        // fetch the messages (GET /sessions/{id}/messages). Auto-marking here
        // sets last_read_seq to MAX(seq) which includes the agent's own
        // messages, so unread_count is always 0 — breaking cross-session
        // unread badges for users who navigated away before the agent finished.

        state2.end_run(&session_id);
        let _ = state2.ws_tx.send(WsEvent::AgentRunDone {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
        });
    });

    // Monitor for panics: if the run task panics, end_run is never called and the
    // session stays stuck. This task catches JoinError and forces cleanup.
    let state_mon = state.clone();
    tokio::spawn(async move {
        if let Err(join_err) = run_handle.await {
            tracing::error!(
                %session_id,
                err = %join_err,
                "agent background task panicked; forcing end_run and AgentError"
            );
            state_mon.end_run(&session_id);
            let _ = state_mon.ws_tx.send(WsEvent::AgentError {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                message: "에이전트가 예기치 않게 종료되었습니다.".to_string(),
            });
        }
    });

    Ok((StatusCode::ACCEPTED, Json(RunAck { status: "started" })))
}
