use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};

use ailoy::{
    agent::{Agent, AgentSpec, AgentState},
    message::{FinishReason, Message, Part, Role},
    runenv::{Machine as _, Sandbox, SandboxNetwork},
};
use chrono::{DateTime, Utc};
use futures_util::{FutureExt as _, StreamExt as _};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{StateError, StateResult, parse_ts, parse_uuid};
use crate::{
    agent_stream::{AgentStreamItem, MessageAssembler},
    event::{EventQueue, RunStatus, SessionEvent, message_channel},
};

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,

    pub workspace_id: Uuid,

    /// The agent definition this session was created from, if any. `None` for
    /// sessions built directly from a preset. Deleting the referenced agent
    /// cascades to this session.
    pub agent_id: Option<Uuid>,

    pub title: Option<String>,

    pub spec: AgentSpec,

    /// Whether this session has a backbone run environment
    pub runenv: bool,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn new(workspace_id: Uuid, spec: AgentSpec) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            agent_id: None,
            title: None,
            spec,
            runenv: false,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_agent_id(mut self, agent_id: Uuid) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_updated_at(mut self) -> Self {
        self.updated_at = Utc::now();
        self
    }

    fn from_sqlite_row(row: &SqliteRow) -> StateResult<Self> {
        let spec_raw: String = row.get("spec");
        let spec: AgentSpec = serde_json::from_str(&spec_raw)
            .map_err(|e| StateError::InvalidData(format!("sessions.spec: {e}")))?;
        let agent_id = row
            .get::<Option<String>, _>("agent_id")
            .map(|raw| parse_uuid(raw, "sessions.agent_id"))
            .transpose()?;
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "sessions.id")?,
            workspace_id: parse_uuid(
                row.get::<String, _>("workspace_id"),
                "sessions.workspace_id",
            )?,
            agent_id,
            title: row.get("title"),
            spec,
            runenv: row.get("runenv"),
            created_at: parse_ts(&row.get::<String, _>("created_at"), "sessions.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "sessions.updated_at")?,
        })
    }
}

/// One persisted history row: the message plus the agent-loop provenance a
/// bare [`Message`] can't carry. This is the JSON shape of `messages.content`;
/// rows written before provenance was recorded are a bare `Message` and are
/// read back as depth-0 with no source (see [`parse_stored_message`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMessage {
    /// Nesting level relative to the top-level agent turn: `0` is the
    /// conversation the user sees; `>= 1` is a sub-agent's internal output.
    /// Only depth-0 rows are replayed into the model's history on the next
    /// run.
    #[serde(default)]
    pub depth: u8,

    /// Name of the agent that produced the message, when known. `None` for
    /// user turns and synthetic (interrupt) messages.
    #[serde(default)]
    pub source_agent: Option<String>,

    pub message: Message,
}

/// Decode one `messages.content` value. New rows store the wrapped
/// [`SessionMessage`] shape; legacy rows are a bare [`Message`] (which lacks
/// the `message` field, so the first parse fails cleanly) and fall back to
/// depth-0 with no source.
fn parse_stored_message(content: &str) -> serde_json::Result<SessionMessage> {
    serde_json::from_str::<SessionMessage>(content).or_else(|_| {
        serde_json::from_str::<Message>(content).map(|message| SessionMessage {
            depth: 0,
            source_agent: None,
            message,
        })
    })
}

/// Book-keeping for one in-flight run, held in [`SessionsState::runs`].
struct ActiveRun {
    /// Wakes the spawned task at its next safe point on
    /// [`SessionsState::cancel`].
    cancel: CancellationToken,

    /// Exact snapshot of the in-progress assistant turn's streamed text,
    /// mirrored from the run task on every delta. Lets a client that attaches
    /// mid-turn catch up without a hole (the SSE handler sends it as one
    /// cumulative delta). A `std` mutex — only ever held for a clone/replace.
    partial: Arc<StdMutex<String>>,
}

pub struct SessionsState {
    db: SqlitePool,

    data_root: PathBuf,

    /// Active runs keyed by session id. The [`CancellationToken`] is held so
    /// [`SessionsState::cancel`] can wake the spawned task; the entry's
    /// presence is also the "is this session running?" gate that rejects
    /// concurrent [`SessionsState::run`] calls.
    runs: Arc<Mutex<HashMap<Uuid, ActiveRun>>>,

    events: EventQueue,
}

impl SessionsState {
    pub fn new(db: SqlitePool, data_root: PathBuf, events: EventQueue) -> Self {
        Self {
            db,
            data_root,
            runs: Arc::new(Mutex::new(HashMap::new())),
            events,
        }
    }

    /// Every session in `workspace_id`, oldest first.
    pub async fn list_by_workspace(&self, workspace_id: Uuid) -> StateResult<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, agent_id, title, spec, runenv, created_at, updated_at \
             FROM sessions WHERE workspace_id = ? ORDER BY created_at ASC",
        )
        .bind(workspace_id.to_string())
        .fetch_all(&self.db)
        .await?;
        rows.iter().map(Session::from_sqlite_row).collect()
    }

    /// Ids of every session owned by `agent_id`. Cheaper than
    /// [`Self::list_by_workspace`] when only the ids are needed — the delete
    /// choreography collects them *before* the agent-row cascade removes the
    /// session rows, so it can still sweep their artifacts afterwards.
    pub async fn ids_by_agent(&self, agent_id: Uuid) -> StateResult<Vec<Uuid>> {
        self.session_ids("agent_id", agent_id).await
    }

    /// Ids of every session in `workspace_id`. See [`Self::ids_by_agent`] for
    /// why the ids are collected ahead of a cascading row delete.
    pub async fn ids_by_workspace(&self, workspace_id: Uuid) -> StateResult<Vec<Uuid>> {
        self.session_ids("workspace_id", workspace_id).await
    }

    /// Session ids where `column` equals `value`. `column` is always an internal
    /// string literal (never user input), so interpolating it is safe.
    async fn session_ids(&self, column: &str, value: Uuid) -> StateResult<Vec<Uuid>> {
        let sql = format!("SELECT id FROM sessions WHERE {column} = ?");
        let rows = sqlx::query(&sql)
            .bind(value.to_string())
            .fetch_all(&self.db)
            .await?;
        rows.iter()
            .map(|r| parse_uuid(r.get::<String, _>("id"), "sessions.id"))
            .collect()
    }

    pub async fn get(&self, id: Uuid) -> StateResult<Option<Session>> {
        let row = sqlx::query(
            "SELECT id, workspace_id, agent_id, title, spec, runenv, created_at, updated_at \
             FROM sessions WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.db)
        .await?;
        row.as_ref().map(Session::from_sqlite_row).transpose()
    }

    /// `INSERT` the session row with its spec persisted as JSON. If a sandbox
    /// is provided, it is stopped and archived into
    /// `data_root/{session_id}/sandbox.tar.zst`; sessions without a sandbox
    /// touch no disk state outside the database. The `runenv` column tracks
    /// whether the archive exists so readers don't need to probe disk.
    pub async fn insert(&self, mut item: Session, runenv: Option<Sandbox>) -> StateResult<()> {
        item.runenv = runenv.is_some();

        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, agent_id, title, spec, runenv, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item.id.to_string())
        .bind(item.workspace_id.to_string())
        .bind(item.agent_id.map(|id| id.to_string()))
        .bind(&item.title)
        .bind(serde_json::to_string(&item.spec)?)
        .bind(item.runenv)
        .bind(item.created_at.to_rfc3339())
        .bind(item.updated_at.to_rfc3339())
        .execute(&self.db)
        .await?;

        if let Some(mut runenv) = runenv {
            let dir = self.data_root.join("sessions").join(item.id.to_string());
            tokio::fs::create_dir_all(&dir).await?;
            runenv
                .stop()
                .await
                .map_err(|e| StateError::Sandbox(format!("{e:#}")))?;
            runenv
                .archive(dir.join("sandbox.tar.zst"))
                .await
                .map_err(|e| StateError::Sandbox(format!("{e:#}")))?;
        }

        Ok(())
    }

    pub async fn remove(&self, id: Uuid) -> StateResult<Session> {
        let existing = self.get(id).await?.ok_or(StateError::NotFound)?;
        // Signal any in-flight run to stop before we tear down the row. The
        // task self-removes from `runs` on its own; we don't wait for it
        // here. A racing INSERT may briefly succeed or fail with an FK
        // error, but the cancellation will close the loop before any
        // further work is done.
        self.cancel(id).await;
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        let dir = self.data_root.join("sessions").join(id.to_string());
        if tokio::fs::try_exists(&dir).await? {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        // Drop the channel so any attached SSE subscribers wake up with
        // RecvError::Closed instead of waiting on a session that no longer
        // exists.
        self.events.remove_channel(&message_channel(id));
        Ok(existing)
    }

    /// Cancel any in-flight run for `id` and remove its on-disk artifacts and
    /// event channel, *without* touching the database — for when the row is
    /// removed elsewhere (e.g. cascaded by a user delete). Best-effort.
    pub async fn discard_artifacts(&self, id: Uuid) {
        self.cancel(id).await;
        let dir = self.data_root.join("sessions").join(id.to_string());
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::error!(session = %id, "failed to remove session dir: {e}");
            }
        }
        self.events.remove_channel(&message_channel(id));
    }

    /// Request that any in-flight run for `id` stop at the next safe point.
    /// Returns `true` if a run was found and signaled, `false` if no run was
    /// active. Non-blocking; the spawned task will clean up its own entry.
    pub async fn cancel(&self, id: Uuid) -> bool {
        match self.runs.lock().await.get(&id) {
            Some(run) => {
                run.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// The streamed-so-far text of the in-progress assistant turn, if a run is
    /// in flight for `id`. `Some("")` means a run is active but hasn't
    /// streamed top-level text yet; `None` means no active run. The SSE
    /// catch-up path uses this so a client attaching mid-turn starts from the
    /// full partial answer instead of the next delta.
    pub async fn live_partial(&self, id: Uuid) -> Option<String> {
        self.runs
            .lock()
            .await
            .get(&id)
            .map(|run| run.partial.lock().expect("partial lock poisoned").clone())
    }

    /// Return all messages for `session_id`, ordered by `seq` ascending. Backs
    /// the `GET /sessions/{id}/messages` endpoint; the SSE catch-up path uses
    /// [`SessionsState::list_messages_since`] instead.
    pub async fn list_messages(&self, session_id: Uuid) -> StateResult<Vec<(i64, SessionMessage)>> {
        self.list_messages_since(session_id, -1).await
    }

    /// Return messages for `session_id` with `seq > since`, ordered ascending.
    /// The SSE handler uses this for catch-up before switching to the live
    /// event subscription.
    pub async fn list_messages_since(
        &self,
        session_id: Uuid,
        since: i64,
    ) -> StateResult<Vec<(i64, SessionMessage)>> {
        let rows = sqlx::query(
            "SELECT seq, content FROM messages \
             WHERE session_id = ? AND seq > ? ORDER BY seq ASC",
        )
        .bind(session_id.to_string())
        .bind(since)
        .fetch_all(&self.db)
        .await?;
        rows.into_iter()
            .map(|r| -> StateResult<(i64, SessionMessage)> {
                let seq: i64 = r.get("seq");
                let content: String = r.get("content");
                Ok((seq, parse_stored_message(&content)?))
            })
            .collect()
    }

    /// Trigger an agent run for `id` with `query` as the user turn. Returns
    /// as soon as the run slot is reserved and the background task is
    /// spawned. Each persisted message is also published on the session's
    /// `message/{id}` channel, along with ephemeral streaming deltas and run
    /// lifecycle events (all no-ops if no one is subscribed). Cancellation is
    /// requested via [`SessionsState::cancel`].
    ///
    /// Returns [`StateError::AlreadyRunning`] if a run is already in flight
    /// for this session.
    pub async fn run(&self, id: Uuid, query: Vec<Part>) -> StateResult<()> {
        let token = CancellationToken::new();
        let partial = Arc::new(StdMutex::new(String::new()));
        {
            let mut runs = self.runs.lock().await;
            if runs.contains_key(&id) {
                return Err(StateError::AlreadyRunning(id));
            }
            runs.insert(
                id,
                ActiveRun {
                    cancel: token.clone(),
                    partial: partial.clone(),
                },
            );
        }

        let db = self.db.clone();
        let data_root = self.data_root.clone();
        let events = self.events.clone();
        let runs = self.runs.clone();

        tokio::spawn(async move {
            let session_key = id.to_string();
            let dir = data_root.join("sessions").join(&session_key);
            let archive_path = dir.join("sandbox.tar.zst");
            let channel = message_channel(id);

            // Ok(bool) is "was the run stopped?" — it picks the terminal
            // lifecycle event published below.
            let result: anyhow::Result<bool> = async {
                events.publish(
                    &channel,
                    serde_json::to_string(&SessionEvent::Run {
                        status: RunStatus::Started,
                    })?,
                );

                // Setup — spec, history, sandbox.
                let row =
                    sqlx::query("SELECT workspace_id, spec, runenv FROM sessions WHERE id = ?")
                        .bind(&session_key)
                        .fetch_optional(&db)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("session {id} not found"))?;
                let workspace_id =
                    parse_uuid(row.get::<String, _>("workspace_id"), "sessions.workspace_id")?;
                let spec: AgentSpec = serde_json::from_str(&row.get::<String, _>("spec"))?;
                let has_runenv: bool = row.get("runenv");

                // The workspace's external-provider mounts, if any. Mounted into
                // the guest below so the agent reads them as files.
                let vfs = crate::state::build_workspace_vfs(&db, workspace_id).await?;

                let runenv = if has_runenv {
                    if !tokio::fs::try_exists(&archive_path).await? {
                        anyhow::bail!(
                            "session {id} marked as having a runenv but archive is missing at {}",
                            archive_path.display()
                        );
                    }
                    // A runenv always runs an in-guest FUSE forwarder (the
                    // unified workspace mount, below), which reaches the host
                    // forward server via host.microsandbox.internal and so needs
                    // guest->host egress. The archive doesn't carry the network
                    // policy, so re-apply it on restore.
                    let sandbox =
                        Sandbox::try_from_archive_with_network(&archive_path, SandboxNetwork::Public).await?;
                    Some(Arc::new(Mutex::new(sandbox)))
                } else {
                    None
                };

                // Mount the unified workspace tree into the guest before the
                // agent runs — local files under `files/` plus the provider
                // mounts as siblings, the browser-WebDAV view served over FUSE
                // at /mnt/workspace. The raw-FUSE tunnel host engine is held for
                // the whole run; dropping it (at the end of this scope) tears the
                // mount down.
                let _vfs_forward = match &runenv {
                    Some(r) => {
                        let mut sandbox = r.lock().await;
                        let console = sandbox.start().await?;
                        let unified: Arc<dyn ::workspace::ForwardFs> = Arc::new(
                            crate::state::workspace_fs(&data_root, workspace_id, vfs.clone())?,
                        );
                        Some(
                            crate::sandbox_tunnel::mount_vfs_tunnel_in_guest(
                                console,
                                unified,
                                "/mnt/workspace",
                                tokio::runtime::Handle::current(),
                            )
                            .await?,
                        )
                    }
                    None => None,
                };

                let rows = sqlx::query(
                    "SELECT seq, content FROM messages WHERE session_id = ? ORDER BY seq ASC",
                )
                .bind(&session_key)
                .fetch_all(&db)
                .await?;
                // Sequence numbers continue from the last persisted row — not
                // from the replayed-history length, which is shorter once
                // sub-agent rows are filtered out below.
                let next_seq_start = rows.last().map(|r| r.get::<i64, _>("seq") + 1).unwrap_or(0);
                // Replay only the top-level conversation (depth 0). A
                // sub-agent's work reaches the model through its capped
                // depth-0 tool result, never its internal transcript —
                // mirroring ailoy's own in-memory history, which only ever
                // holds depth-0 messages.
                let history: Vec<Message> = rows
                    .iter()
                    .map(|r| parse_stored_message(&r.get::<String, _>("content")))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter(|stored| stored.depth == 0)
                    .map(|stored| stored.message)
                    .collect();

                // Drive — stream the model's raw deltas, publishing live text
                // fragments as they arrive and persisting each message at its
                // boundary. The sandbox archive below must run regardless of
                // how this exits, so capture the drive's Result and propagate
                // it after archiving.
                let drive: anyhow::Result<bool> = async {
                    let mut next_seq = next_seq_start;
                    let mut state = AgentState::new().with_history(history);
                    if let Some(ref r) = runenv {
                        state = state.with_runenv(r.clone());
                    }
                    let mut agent = Agent::try_with_state(spec, state)?;

                    let user_msg = Message::new(Role::User).with_contents(query);
                    persist_message(
                        &db,
                        &events,
                        &channel,
                        &session_key,
                        next_seq,
                        SessionMessage {
                            depth: 0,
                            source_agent: None,
                            message: user_msg.clone(),
                        },
                    )
                    .await?;
                    next_seq += 1;

                    // `run_stream` yields raw `MessageDeltaOutput`s; the
                    // assembler reassembles them into live assistant deltas +
                    // completed messages (boundary detection and the
                    // trailing-flush quirk live in `agent_stream`, unit-tested
                    // there).
                    let mut assembler = MessageAssembler::new();
                    // Streamed text of the in-flight assistant turn. ailoy drops
                    // its own accumulator when the stream is dropped, so if a
                    // stop cuts a turn short before it commits a message, this
                    // is the only copy of the partial answer. Cleared when a
                    // top-level assistant message commits.
                    let mut partial_text = String::new();
                    // Tool-call ids still awaiting a Tool result, so a stop can
                    // persist stub results — a replayed history with a tool
                    // call but no result is rejected by most providers.
                    let mut pending_calls: Vec<String> = Vec::new();
                    // Whether the last committed top-level output ended the
                    // turn (assistant + Stop) — a cancel landing at that exact
                    // instant is a normal completion, not an interruption.
                    let mut completed_naturally = false;
                    let mut stopped = false;
                    let mut run_error: Option<String> = None;

                    let mut stream = agent.run_stream(user_msg);
                    loop {
                        let item = if stopped {
                            // Drain mode (entered on cancel). Take only what
                            // ailoy has already produced, never awaiting new
                            // output, so a turn that finished at the exact
                            // cancel instant still commits its terminal message
                            // instead of being mistagged as interrupted.
                            // Nothing ready → break, dropping the stream (which
                            // aborts the in-flight request), so stop stays
                            // prompt.
                            match stream.next().now_or_never() {
                                Some(item) => item,
                                None => break,
                            }
                        } else {
                            tokio::select! {
                                biased;
                                _ = token.cancelled() => {
                                    stopped = true;
                                    continue;
                                }
                                item = stream.next() => item,
                            }
                        };
                        // `last` marks the trailing-flush pseudo-item so the
                        // loop exits after committing it.
                        let (produced, last) = match item {
                            Some(Ok(delta)) => match assembler.push(delta) {
                                Ok(items) => (items, false),
                                Err(e) => {
                                    run_error = Some(e);
                                    break;
                                }
                            },
                            Some(Err(e)) => {
                                run_error = Some(format!("{e:#}"));
                                break;
                            }
                            // A stopped turn's pending partial is handled by
                            // the interrupt path below, not flushed here.
                            None if stopped => break,
                            None => match assembler.finish() {
                                // Defensive trailing flush: ailoy's contract
                                // (every message ends with a finish_reason
                                // delta) makes this a no-op for conforming
                                // streams — a message here means an upstream
                                // producer broke the contract. Still commit it
                                // (losing a completed answer is worse) but warn
                                // so the violation is visible.
                                Ok(Some(output)) => {
                                    tracing::warn!(
                                        session = %id,
                                        "stream ended mid-message (contract violation upstream); committing trailing message"
                                    );
                                    (vec![AgentStreamItem::Completed(Box::new(output))], true)
                                }
                                Ok(None) => break,
                                Err(e) => {
                                    tracing::warn!(session = %id, "failed to finalize trailing stream delta: {e}");
                                    break;
                                }
                            },
                        };
                        for out in produced {
                            match out {
                                // Live assistant text. Published incrementally
                                // with the running total so the client can dedup
                                // at the catch-up↔live boundary; the shared
                                // `partial` snapshot backs mid-turn
                                // (re)subscribes.
                                AgentStreamItem::Delta(fragment) => {
                                    partial_text.push_str(&fragment);
                                    // UTF-16 units so cum_len matches the
                                    // client's String.length.
                                    let cum_len = partial_text.encode_utf16().count() as u64;
                                    *partial.lock().expect("partial lock poisoned") =
                                        partial_text.clone();
                                    events.publish(
                                        &channel,
                                        serde_json::to_string(&SessionEvent::Delta {
                                            text: fragment,
                                            cum_len,
                                        })?,
                                    );
                                }
                                // A message finalized at its boundary — persist
                                // and publish it, and update the stop
                                // book-keeping for top-level outputs.
                                AgentStreamItem::Completed(output) => {
                                    let output = *output;
                                    if matches!(output.depth, None | Some(0)) {
                                        match output.message.role {
                                            Role::Assistant => {
                                                partial_text.clear();
                                                partial
                                                    .lock()
                                                    .expect("partial lock poisoned")
                                                    .clear();
                                                completed_naturally = matches!(
                                                    output.finish_reason,
                                                    FinishReason::Stop {}
                                                );
                                                for tc in output.message.tool_calls.iter().flatten()
                                                {
                                                    if let Some((call_id, _, _)) = tc.as_function()
                                                    {
                                                        pending_calls.push(call_id.to_string());
                                                    }
                                                }
                                            }
                                            Role::Tool => {
                                                completed_naturally = false;
                                                if let Some(call_id) = output.message.id.as_deref()
                                                {
                                                    pending_calls.retain(|c| c != call_id);
                                                }
                                            }
                                            _ => completed_naturally = false,
                                        }
                                    }
                                    persist_message(
                                        &db,
                                        &events,
                                        &channel,
                                        &session_key,
                                        next_seq,
                                        SessionMessage {
                                            depth: output.depth.unwrap_or(0),
                                            source_agent: output.source_agent,
                                            message: output.message,
                                        },
                                    )
                                    .await?;
                                    next_seq += 1;
                                }
                            }
                        }
                        if last {
                            break;
                        }
                    }
                    drop(stream);

                    if let Some(err) = run_error {
                        if stopped {
                            // The error occurred during the grace period after a
                            // stop request. Treat it as a clean stop rather than
                            // a hard failure: keep the partial outputs already
                            // committed instead of surfacing an error.
                            tracing::warn!(
                                session = %id,
                                "agent error during stop grace period (treating as stop): {err}"
                            );
                        } else {
                            anyhow::bail!("{err}");
                        }
                    }

                    if stopped {
                        if completed_naturally {
                            // Cancel fired at the exact moment the turn
                            // finished; report a normal completion so the
                            // client doesn't see a spurious "stopped".
                            stopped = false;
                        } else {
                            for call_id in std::mem::take(&mut pending_calls) {
                                let stub = Message::new(Role::Tool)
                                    .with_id(call_id)
                                    .with_contents([Part::text(
                                        "[Interrupted: the user stopped response generation before this tool call completed]",
                                    )]);
                                persist_message(
                                    &db,
                                    &events,
                                    &channel,
                                    &session_key,
                                    next_seq,
                                    SessionMessage {
                                        depth: 0,
                                        source_agent: None,
                                        message: stub,
                                    },
                                )
                                .await?;
                                next_seq += 1;
                            }
                            const INTERRUPT_NOTE: &str =
                                "[Interrupted: the user manually stopped response generation here]";
                            let note = if partial_text.is_empty() {
                                INTERRUPT_NOTE.to_string()
                            } else {
                                // The interrupted turn streamed text but never
                                // committed a message. Persist what accumulated
                                // so the partial answer survives a refresh.
                                format!("{partial_text}\n\n{INTERRUPT_NOTE}")
                            };
                            persist_message(
                                &db,
                                &events,
                                &channel,
                                &session_key,
                                next_seq,
                                SessionMessage {
                                    depth: 0,
                                    source_agent: None,
                                    message: Message::new(Role::Assistant)
                                        .with_contents([Part::text(note)]),
                                },
                            )
                            .await?;
                            partial.lock().expect("partial lock poisoned").clear();
                        }
                    }

                    Ok(stopped)
                }
                .await;

                if let Some(runenv) = runenv {
                    let mut sandbox = runenv.lock().await;
                    let archive: anyhow::Result<()> = async {
                        sandbox.stop().await?;
                        if tokio::fs::try_exists(&archive_path).await? {
                            tokio::fs::remove_file(&archive_path).await?;
                        }
                        sandbox.archive(&archive_path).await?;
                        Ok(())
                    }
                    .await;
                    if let Err(e) = archive {
                        tracing::error!(session = %id, "sandbox archive failed: {e:#}");
                    }
                }

                drive
            }
            .await;

            let status = match &result {
                Ok(false) => RunStatus::Done,
                Ok(true) => RunStatus::Stopped,
                Err(e) => {
                    tracing::error!(session = %id, "run failed: {e:#}");
                    RunStatus::Error {
                        message: format!("{e:#}"),
                    }
                }
            };
            match serde_json::to_string(&SessionEvent::Run { status }) {
                Ok(payload) => {
                    events.publish(&channel, payload);
                }
                Err(e) => tracing::error!(session = %id, "failed to serialize run status: {e}"),
            }
            runs.lock().await.remove(&id);
        });

        Ok(())
    }
}

/// `INSERT` one message row for `session_key` at `seq` (stored in the wrapped
/// [`SessionMessage`] shape) and publish it on the session's channel (a no-op
/// when nobody is subscribed).
async fn persist_message(
    db: &SqlitePool,
    events: &EventQueue,
    channel: &str,
    session_key: &str,
    seq: i64,
    stored: SessionMessage,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO messages (session_id, seq, content, created_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(session_key)
    .bind(seq)
    .bind(serde_json::to_string(&stored)?)
    .bind(Utc::now().to_rfc3339())
    .execute(db)
    .await?;
    events.publish(
        channel,
        serde_json::to_string(&SessionEvent::Message {
            seq,
            depth: stored.depth,
            source_agent: stored.source_agent,
            message: stored.message,
        })?,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows written before provenance was recorded are a bare `Message`; they
    /// must keep parsing (as depth-0, no source) alongside the wrapped shape,
    /// or existing sessions break on upgrade.
    #[test]
    fn stored_message_parses_wrapped_and_legacy_rows() {
        let wrapped = SessionMessage {
            depth: 1,
            source_agent: Some("researcher".into()),
            message: Message::new(Role::Assistant).with_contents([Part::text("found it")]),
        };
        let parsed = parse_stored_message(&serde_json::to_string(&wrapped).unwrap()).unwrap();
        assert_eq!(parsed.depth, 1);
        assert_eq!(parsed.source_agent.as_deref(), Some("researcher"));
        assert_eq!(parsed.message.contents[0].as_text(), Some("found it"));

        let legacy = Message::new(Role::User).with_contents([Part::text("hi")]);
        let parsed = parse_stored_message(&serde_json::to_string(&legacy).unwrap()).unwrap();
        assert_eq!(parsed.depth, 0);
        assert!(parsed.source_agent.is_none());
        assert_eq!(parsed.message.role, Role::User);
    }
}
