use std::{collections::HashSet, sync::Arc};

use agent_k::agents::{GUEST_ATTACHED_DIR, GUEST_SHARED_DIR};
use ailoy::{
    agent::{Agent, AgentEvent},
    message::{FinishReason, Message, MessageOutput, Part, PartDelta, Role},
    runenv::{Sandbox, SandboxConfig},
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use futures_util::{FutureExt, StreamExt};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiResult, AppError},
    events::{RunUserMessage, WsEvent},
    handlers::dirent::{
        DirentScope, copy_dir_recursive, enforce_scope_access, parse_dirent_path, scope_root,
    },
    model::{
        ActiveRunOutput, ActiveRunSnapshot, CreateSessionRequest, MessageSender, RunAck,
        RunActiveResponse, SendMessageRequest, SessionListResponse, SessionMessageListResponse,
        SessionMessageResponse, SessionResponse, UpdateSessionRequest,
    },
    repository::{
        DbSenderKind, NewSessionMessage, PrefixLookup, SessionAccess, SessionOrigin, ShareMode,
    },
    services::session_title::generate_session_title,
    state::AppState,
};

const TOP_LEVEL_AGENT_NAME: &str = "agent-k";
const SANDBOX_IMAGE: &str = "brekkylab/agent-k:latest";

pub(crate) fn sandbox_name_for(id: &Uuid) -> String {
    let s = id.simple().to_string();
    format!("session-{}", &s[..12])
}

/// Open the project's corpus store for use as a sub-agent, logging (not
/// swallowing) a failure. A `None` here means Coworker/Deep Research run without
/// corpus access; logging makes that visible instead of a silent capability loss.
async fn corpus_store_or_log(
    state: &AppState,
    project_id: Uuid,
) -> Option<agent_k::knowledge_base::SharedStore> {
    match state.store_for(project_id).await {
        Ok(store) => Some(store),
        Err((status, _)) => {
            tracing::warn!(%project_id, "corpus store unavailable ({status}); sub-agent will run without corpus access");
            None
        }
    }
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
    let pin = model_pin.as_deref();
    let model = crate::model::resolve_model_in(&chain, pin);

    use crate::model::AgentType;
    let agent = match agent_type {
        AgentType::DeepResearch => {
            // Attach a Speedwagon sub-agent when the project's corpus store
            // opens; on failure, run without it rather than failing the session
            // (but log it — a broken store silently drops corpus access).
            let corpus = corpus_store_or_log(&*state, project_id).await;
            // Run the Speedwagon sub-agent on the corpus-recommended model of the
            // same provider, not necessarily Deep Research's own model.
            let corpus_model = corpus
                .is_some()
                .then(|| crate::model::speedwagon_model_for_parent(&model));
            agent_k::agents::get_deep_research_agent(
                TOP_LEVEL_AGENT_NAME,
                &model,
                &artifacts,
                corpus,
                corpus_model,
            )
            .await
            .map_err(|e| e.to_string())?
        }
        AgentType::Buddy => agent_k::agents::get_buddy_agent(TOP_LEVEL_AGENT_NAME, &model)
            .map_err(|e| e.to_string())?,
        // Speedwagon answers questions over this project's document corpus.
        // Tools bind to the project-scoped store; runs on a local RunEnv (not
        // the session sandbox). Shell is exposed as a secondary tool.
        AgentType::Speedwagon => {
            let store = state
                .store_for(project_id)
                .await
                .map_err(|(status, _)| format!("failed to open document store ({status})"))?;
            agent_k::agents::get_speedwagon_agent(TOP_LEVEL_AGENT_NAME, &model, store, true)
                .await
                .map_err(|e| e.to_string())?
        }
        // Coworker runs the sandboxed coworker agent over the session's files.
        AgentType::Coworker => {
            // Attach a Speedwagon sub-agent when the project's corpus store
            // opens; on failure, run without it rather than failing the session
            // (but log it — a broken store silently drops corpus access).
            let corpus_store = corpus_store_or_log(&*state, project_id).await;
            // Run the Speedwagon sub-agent on the corpus-recommended model of the
            // same provider, not necessarily Coworker's own model.
            let corpus_model = corpus_store
                .is_some()
                .then(|| crate::model::speedwagon_model_for_parent(&model));
            let opts = agent_k::agents::CoworkerSandboxOptions {
                sandbox_name: Some(sandbox_name_for(&session_id)),
                persist: true,
                with_skill: true,
                corpus_store,
                corpus_model,
            };
            agent_k::agents::get_coworker_agent_with_opts(
                TOP_LEVEL_AGENT_NAME,
                &model,
                &inputs,
                &shared,
                &artifacts,
                opts,
            )
            .await
            .map_err(|e| e.to_string())?
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

/// Hard ceiling on attachments per message — a server-side backstop for the
/// frontend's softer limit. Bounds the agent prompt, DB row, and fs stats even
/// if a crafted request bypasses the client.
const MAX_ATTACHMENTS: usize = 30;

/// Reject a message that attaches more than [`MAX_ATTACHMENTS`] files.
pub fn ensure_attachment_count(count: usize) -> ApiResult<()> {
    if count > MAX_ATTACHMENTS {
        return Err(AppError::bad_request(format!(
            "too many attachments: {count} (max {MAX_ATTACHMENTS})"
        )));
    }
    Ok(())
}

async fn validate_attachments(
    state: &Arc<AppState>,
    auth_user: &AuthUser,
    session_id: Uuid,
    project_id: Uuid,
    attachments: &[String],
) -> ApiResult<()> {
    ensure_attachment_count(attachments.len())?;
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
    /// Filter by session origin (`user` or `automation`). Omit to list all.
    pub origin: Option<SessionOrigin>,
}

/// GET /sessions?project_ref=...&origin=...
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
                .list_sessions_in_project(project_id, auth_user.id, q.origin)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?
        }
        None => state
            .repository
            .list_sessions_for_user(auth_user.id, q.origin)
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

pub(crate) async fn cleanup_session_resources(
    state: &Arc<AppState>,
    project_id: Uuid,
    session_id: Uuid,
) {
    state.remove_agent(&session_id);
    state.clear_session_citation_checks(session_id);
    let sandbox_name = sandbox_name_for(&session_id);
    if let Err(e) = Sandbox::remove_persisted(&sandbox_name).await {
        tracing::warn!(%session_id, "failed to remove persisted sandbox: {e}");
    }
    let session_rt = session_root(state, project_id, session_id);
    if let Err(e) = tokio::fs::remove_dir_all(&session_rt).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(%session_id, "failed to remove session dir: {e}");
        }
    }
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

    cleanup_session_resources(&state, session.project_id, session_id).await;

    tracing::info!(%session_id, "session deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// POST /sessions/{session_id}/fork
pub async fn fork_session(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(source_session_ref): Path<String>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let source_session_id = resolve_session_id(&state, &source_session_ref).await?;
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

    // Mirror the source session's host files into the fork so the snapshot matches
    // the VM upper.ext4 clone. shared/ is project-level and intentionally excluded.
    let (src_inputs, _, src_artifacts) = session_dirs(&state, source.project_id, source_session_id);
    let (new_inputs, _, new_artifacts) = session_dirs(&state, source.project_id, new_id);
    for (src, dst) in [(&src_inputs, &new_inputs), (&src_artifacts, &new_artifacts)] {
        let res = if tokio::fs::try_exists(src).await.unwrap_or(false) {
            copy_dir_recursive(src, dst).await
        } else {
            tokio::fs::create_dir_all(dst).await
        };
        if let Err(e) = res {
            tracing::warn!(
                source = %source_session_id,
                fork = %new_id,
                dir = ?dst,
                "failed to copy session dir on fork: {e}",
            );
        }
    }

    let source_sandbox_name = sandbox_name_for(&source_session_id);

    // Only fork the sandbox when the source session actually has one.
    // If the source never had a sandbox (e.g. DeepResearch / Buddy / Speedwagon,
    // or a Coworker session that was created but never run), skip the VM fork
    // entirely — the forked session will create its own sandbox on first run.
    if !Sandbox::exists(&source_sandbox_name).await {
        tracing::info!(
            source = %source_session_id,
            fork = %new_id,
            project = %source.project_id,
            "session forked without sandbox (source has none)",
        );
        return Ok((
            StatusCode::CREATED,
            Json(SessionResponse::from_db(new_session, 0)),
        ));
    }

    let source_cfg = SandboxConfig {
        name: Some(source_sandbox_name),
        image: SANDBOX_IMAGE.into(),
        persist: true,
        ..Default::default()
    };
    let source_sandbox = match Sandbox::new(source_cfg).await {
        Ok(s) => s,
        Err(e) => {
            let _ = state.repository.delete_session(new_id).await;
            // Clean up pre-created session dirs
            let session_rt = session_root(&state, source.project_id, new_id);
            let _ = tokio::fs::remove_dir_all(&session_rt).await;
            return Err(AppError::internal(e.to_string()));
        }
    };

    let new_sandbox_name = sandbox_name_for(&new_id);
    let new_cfg = SandboxConfig {
        name: Some(new_sandbox_name.clone()),
        image: SANDBOX_IMAGE.into(),
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
            let _ = Sandbox::remove_persisted(&new_sandbox_name).await;
            let session_rt = session_root(&state, source.project_id, new_id);
            let _ = tokio::fs::remove_dir_all(&session_rt).await;
            Err(AppError::internal(format!("sandbox fork failed: {e}")))
        }
    }
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields, default)]
pub struct MessageHistoryQuery {
    /// Max TURNS below the cursor. Omit for everything; ignored with `after_seq`.
    pub limit: Option<u32>,
    /// Keyset cursor for older pages: only messages with `seq < before_seq`.
    pub before_seq: Option<i64>,
    /// Tail catch-up: messages with `seq > after_seq`. Exclusive with `before_seq`.
    pub after_seq: Option<i64>,
}

/// GET /sessions/{session_id}/messages?limit=...&before_seq=...|after_seq=...
/// Items are returned in ascending (oldest→newest) order within the window.
pub async fn get_message_history(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_ref): Path<String>,
    axum::extract::Query(q): axum::extract::Query<MessageHistoryQuery>,
) -> ApiResult<Json<SessionMessageListResponse>> {
    if q.before_seq.is_some() && q.after_seq.is_some() {
        return Err(AppError::bad_request(
            "before_seq and after_seq are mutually exclusive",
        ));
    }

    let session_id = resolve_session_id(&state, &session_ref).await?;
    let (session, _access) = state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    let rows = match q.after_seq {
        Some(after_seq) => state
            .repository
            .get_messages_after(session_id, after_seq)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
        None => state
            .repository
            .get_messages_window(session_id, q.limit, q.before_seq)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    };

    // For a Speedwagon session, the corpus (title, line_count) summary backs the
    // footnote citation checks. Read the cache (refreshed by resync); only on a
    // cold cache compute it once from the store and warm it, so the message
    // fetch doesn't load every document's content on every poll.
    let corpus_docs: Vec<(String, usize)> = if session.agent_type.as_deref() == Some("speedwagon") {
        match state.corpus_summary(session.project_id) {
            Some(summary) => (*summary).clone(),
            None => match state.store_for(session.project_id).await {
                Ok(store) => {
                    let summary = crate::handlers::knowledge::corpus_summary(&*store.read().await);
                    state.set_corpus_summary(session.project_id, summary.clone());
                    summary
                }
                Err(_) => Vec::new(),
            },
        }
    } else {
        Vec::new()
    };

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
            // Only Speedwagon answers carry corpus citations to check. Reuse a
            // cached result when present; otherwise verify once and cache it.
            // The cache is dropped per project whenever the corpus changes, so a
            // hit always reflects the current corpus.
            let citations =
                if matches!(r.sender_kind, DbSenderKind::Agent) && !corpus_docs.is_empty() {
                    match state.citation_checks(session.project_id, session.id, r.seq) {
                        Some(cached) => (*cached).clone(),
                        None => {
                            let text: String = r
                                .message
                                .contents
                                .iter()
                                .filter_map(|p| p.as_text())
                                .collect();
                            let checks =
                                crate::handlers::knowledge::verify_citations(&text, &corpus_docs);
                            state.set_citation_checks(
                                session.project_id,
                                session.id,
                                r.seq,
                                checks.clone(),
                            );
                            checks
                        }
                    }
                } else {
                    Vec::new()
                };
            Ok(SessionMessageResponse {
                seq: r.seq,
                message: r.message,
                sender,
                created_at: r.created_at,
                attachments: r.attachments,
                artifacts: r.artifacts,
                citations,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Mark read only when the client is at the tail (no before_seq) —
    // paging older history must not clear unread.
    if q.before_seq.is_none() {
        let _ = state
            .repository
            .mark_session_read(session_id, auth_user.id)
            .await;
    }

    Ok(Json(SessionMessageListResponse { items }))
}

/// POST /sessions/{session_id}/read — mark the session read without fetching
/// history (e.g. the sidebar "Mark as read" action).
pub async fn mark_session_read(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_ref): Path<String>,
) -> ApiResult<StatusCode> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
    state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    state
        .repository
        .mark_session_read(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
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
    let (run_id, cancel) = state.start_run(session_id, user_message.clone());
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
        let mut run = agent.run_stream(msg);

        let mut run_error: Option<String> = None;
        let mut stopped = false;
        let mut depth0_outputs: Vec<MessageOutput> = Vec::new();
        // Accumulates streamed assistant text for the in-flight turn. ailoy drops
        // its own accumulator when the stream is dropped, so if a stop cuts a turn
        // short before it commits a Message, this is the only copy of the partial
        // answer. Cleared whenever a depth-0 assistant Message commits.
        let mut partial_text = String::new();
        loop {
            let item = if stopped {
                // Drain mode (entered on cancel). Take only what ailoy has already
                // produced, never awaiting new output, so a turn that finished at
                // the exact cancel instant still commits its terminal Message
                // (→ completed_naturally) instead of being mistagged [Interrupted].
                // Nothing ready → drop the stream below, aborting the in-flight
                // request, so stop stays prompt.
                match run.next().now_or_never() {
                    Some(item) => item, // Some(x) ready, or None if the stream ended
                    None => break,      // would block → nothing buffered, stop now
                }
            } else {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        // Switch to drain mode rather than breaking immediately, so
                        // an already-produced terminal Message isn't discarded.
                        // Whatever text streamed so far is in `partial_text` and is
                        // persisted below if the turn truly didn't finish.
                        stopped = true;
                        continue;
                    }
                    item = run.next() => item,
                }
            };
            match item {
                // Live text fragment. Broadcast it incrementally (cheap — small
                // payload even with many subscribers) with the running total so
                // the client can dedup at the replay↔live boundary. Keep the full
                // text in `partial_text` for stop-persist and mid-turn resume.
                // Ephemeral — not persisted, no seq.
                Some(Ok(AgentEvent::Delta(delta))) => {
                    let mut fragment = String::new();
                    for part in &delta.delta.contents {
                        if let PartDelta::Text { text } = part {
                            fragment.push_str(text);
                        }
                    }
                    if !fragment.is_empty() {
                        partial_text.push_str(&fragment);
                        // UTF-16 units so cum_len matches the client's String.length.
                        let cum_len = partial_text.encode_utf16().count() as u64;
                        // Keep the resume snapshot exact so a (re)subscribe never
                        // lands in a hole (cheap: in-process, not per-subscriber).
                        state2.set_partial(&session_id, partial_text.clone()).await;
                        let _ = state2.ws_tx.send(WsEvent::AgentDelta {
                            session_id: session_id.to_string(),
                            run_id: run_id.to_string(),
                            delta: fragment,
                            cum_len,
                        });
                    }
                }
                // Completed turn: identical to what `run()` used to yield.
                Some(Ok(AgentEvent::Message(output))) => {
                    if matches!(output.depth, None | Some(0)) {
                        if output.message.role == Role::Assistant {
                            // This turn committed as a real Message; the partial
                            // buffer is now redundant — clear it so the next turn
                            // accumulates from empty (and a later stop doesn't
                            // re-persist already-committed text). Also clear the
                            // replayable snapshot so a resume doesn't double it.
                            partial_text.clear();
                            state2.set_partial(&session_id, String::new()).await;
                        }
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
                    // No early break in drain mode: keep taking already-ready items
                    // until the stream ends (`None`) or nothing is buffered
                    // (`now_or_never` → break above), so a finish-line turn commits.
                }
                Some(Err(e)) => {
                    run_error = Some(e.to_string());
                    break; // Must break before accessing `agent` — `run` borrows it
                }
                None => break,
            }
        }
        drop(run);

        if let Some(err) = run_error {
            if stopped {
                // The error occurred during the grace period after the user requested a stop.
                // Treat it as a clean stop rather than a hard failure: keep whatever partial
                // outputs were already accumulated instead of rolling back the entire turn.
                tracing::warn!(%session_id, "agent error during stop grace period (treating as stop): {err}");
            } else {
                tracing::error!(%session_id, "agent run failed: {err}");
                // Truncate in-memory history to match DB state so the agent stays consistent.
                agent.state.history.truncate(prev_len);
                // End the run before releasing the lock so no newer run can slot
                // in between drop and cleanup.
                state2.end_run(&session_id, run_id);
                drop(agent);
                let _ = state2.ws_tx.send(WsEvent::AgentError {
                    session_id: session_id.to_string(),
                    run_id: run_id.to_string(),
                    message: err,
                });
                return;
            }
        }

        let mut new_msgs = agent.get_history()[prev_len..].to_vec();

        let mut synthetic_msgs: Vec<Message> = Vec::new();
        if stopped {
            let answered: HashSet<&str> = new_msgs
                .iter()
                .filter(|m| m.role == Role::Tool)
                .filter_map(|m| m.id.as_deref())
                .collect();
            for m in new_msgs.iter().filter(|m| m.role == Role::Assistant) {
                for tc in m.tool_calls.iter().flatten() {
                    if let Some((call_id, _, _)) = tc.as_function() {
                        if !answered.contains(call_id) {
                            synthetic_msgs.push(
                                Message::new(Role::Tool).with_id(call_id).with_contents([
                                    Part::text(
                                        "[Interrupted: the user stopped response generation before this tool call completed]",
                                    ),
                                ]),
                            );
                        }
                    }
                }
            }
            // Skip the note if the model finished its turn anyway (last output is an
            // assistant message with FinishReason::Stop) — not truncated.
            let completed_naturally = matches!(
                depth0_outputs.last(),
                Some(o) if o.message.role == Role::Assistant
                    && matches!(o.finish_reason, FinishReason::Stop {})
            );
            if completed_naturally {
                // Cancel fired at the exact moment the model finished; treat as a normal
                // completion so the client doesn't see a spurious "Run stopped" toast.
                stopped = false;
            } else {
                // Mark the turn as cut short.
                const INTERRUPT_NOTE: &str =
                    "[Interrupted: the user manually stopped response generation here]";
                if !partial_text.is_empty() {
                    // The interrupted turn streamed text but never committed a
                    // Message (ailoy dropped its accumulator). Persist what we
                    // accumulated as a fresh assistant message so the partial
                    // answer survives a refresh. Routed through synthetic_msgs so
                    // it is both persisted and pushed into history (keeping the
                    // agent's in-memory state consistent with the DB).
                    partial_text.push_str("\n\n");
                    partial_text.push_str(INTERRUPT_NOTE);
                    synthetic_msgs.push(
                        Message::new(Role::Assistant).with_contents([Part::text(partial_text)]),
                    );
                } else if let Some(last) = new_msgs
                    .iter_mut()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                {
                    last.contents.push(Part::text(INTERRUPT_NOTE));
                    if let Some(warm) = agent.state.history[prev_len..]
                        .iter_mut()
                        .rev()
                        .find(|m| m.role == Role::Assistant)
                    {
                        warm.contents.push(Part::text(INTERRUPT_NOTE));
                    }
                } else {
                    synthetic_msgs.push(
                        Message::new(Role::Assistant).with_contents([Part::text(INTERRUPT_NOTE)]),
                    );
                }
            }
            agent.state.history.extend(synthetic_msgs.iter().cloned());
        }

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

        to_persist.extend(synthetic_msgs.into_iter().map(|message| NewSessionMessage {
            message,
            sender_kind: crate::repository::DbSenderKind::Agent,
            sender_name: Some(TOP_LEVEL_AGENT_NAME.to_string()),
            sender_user_id: None,
            attachments: vec![],
            artifacts: vec![],
        }));

        // [A] Persist before releasing the agent lock. On failure: roll back in-memory history,
        // send AgentError (not AgentRunDone), and return — the DB remains consistent.
        if let Err(e) = state2
            .repository
            .append_messages(session_id, &to_persist)
            .await
        {
            tracing::error!(%session_id, "failed to persist messages: {e}");
            agent.state.history.truncate(prev_len);
            state2.end_run(&session_id, run_id);
            drop(agent);
            let _ = state2.ws_tx.send(WsEvent::AgentError {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                message: "응답 저장에 실패했습니다. 다시 시도해주세요.".to_string(),
            });
            return;
        }
        // End the run before releasing the lock: while the OwnedMutexGuard is
        // held no other request can acquire it and start a replacement run, so
        // this can never delete a newer run's record.
        state2.end_run(&session_id, run_id);
        drop(agent); // Release OwnedMutexGuard only after successful persist + cleanup

        // Intentionally NOT calling mark_session_read here: the sender should
        // only be considered to have read the agent's reply when they actually
        // fetch the messages (GET /sessions/{id}/messages). Auto-marking here
        // sets last_read_seq to MAX(seq) which includes the agent's own
        // messages, so unread_count is always 0 — breaking cross-session
        // unread badges for users who navigated away before the agent finished.

        let _ = state2.ws_tx.send(WsEvent::AgentRunDone {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            stopped,
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
            // run_id-guarded: unwind released the agent lock before this task
            // ran, so a newer run may already own the session's entry — only
            // remove ours.
            state_mon.end_run(&session_id, run_id);
            let _ = state_mon.ws_tx.send(WsEvent::AgentError {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                message: "에이전트가 예기치 않게 종료되었습니다.".to_string(),
            });
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(RunAck {
            status: "started",
            run_id: run_id.to_string(),
        }),
    ))
}

/// POST /sessions/{session_id}/runs/{run_id}/stop
pub async fn stop_run(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((session_ref, run_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
    state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    let run_id = Uuid::parse_str(&run_id).map_err(|_| AppError::not_found("no such run"))?;

    let Some((active_run_id, sender_user_id, cancel)) = state.run_cancel_info(&session_id).await
    else {
        return Err(AppError::not_found("no active run for this session"));
    };
    if active_run_id != run_id {
        return Err(AppError::not_found("run is not active"));
    }
    if sender_user_id != auth_user.id.to_string() {
        return Err(AppError::forbidden(
            "only the user who started the run can stop it",
        ));
    }

    cancel.cancel();
    tracing::info!(%session_id, %run_id, user = %auth_user.id, "run cancellation requested");
    Ok(StatusCode::ACCEPTED)
}

/// GET /sessions/{session_id}/runs/{run_id}/active
pub async fn run_active(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((session_ref, run_id)): Path<(String, String)>,
) -> ApiResult<Json<RunActiveResponse>> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
    state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    let run_id = Uuid::parse_str(&run_id).map_err(|_| AppError::not_found("no such run"))?;

    let active = matches!(
        state.run_cancel_info(&session_id).await,
        Some((active_run_id, _, _)) if active_run_id == run_id
    );
    Ok(Json(RunActiveResponse { active }))
}

/// Snapshot of the session's in-flight run (or `null` if none), so a client
/// loading the page mid-stream can render the in-progress turn immediately
/// rather than waiting for the WebSocket replay. The completed run is in the DB
/// via the normal message history; this only covers the not-yet-persisted run.
pub async fn get_active_run(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(session_ref): Path<String>,
) -> ApiResult<Json<Option<ActiveRunSnapshot>>> {
    let session_id = resolve_session_id(&state, &session_ref).await?;
    state
        .repository
        .get_session_with_authz(session_id, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("session not found or access denied"))?;

    let snapshot = state.snapshot(&session_id).await.map(
        |(run_id, user_message, outputs, partial)| ActiveRunSnapshot {
            run_id: run_id.to_string(),
            user_message,
            outputs: outputs
                .into_iter()
                .map(|(seq, output)| ActiveRunOutput { seq, output })
                .collect(),
            partial,
        },
    );
    Ok(Json(snapshot))
}
