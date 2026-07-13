use std::{collections::HashMap, path::PathBuf, sync::Arc};

use ailoy::{
    agent::{Agent, AgentSpec, AgentState},
    message::{Message, Part, Role},
    runenv::{Machine as _, Sandbox},
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{StateError, StateResult, parse_ts, parse_uuid};
use crate::event::{EventQueue, MessageEvent, RunEvent, TitleEvent, message_channel};

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

/// Concatenate the text parts of a user turn for title generation. Non-text
/// parts (images, tool calls) are skipped.
fn first_user_text(parts: &[Part]) -> String {
    parts.iter().filter_map(|p| p.as_text()).collect()
}

pub struct SessionsState {
    db: SqlitePool,

    data_root: PathBuf,

    /// Active runs keyed by session id. The [`CancellationToken`] is held so
    /// [`SessionsState::cancel`] can wake the spawned task; the entry's
    /// presence is also the "is this session running?" gate that rejects
    /// concurrent [`SessionsState::run`] calls.
    runs: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,

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

    /// Absolute on-disk path of session `id`'s directory
    /// (`data_root/sessions/{id}`), holding the sandbox archive and the
    /// inputs/artifacts bind-mount roots.
    pub fn session_dir(&self, id: Uuid) -> PathBuf {
        self.data_root.join("sessions").join(id.to_string())
    }

    /// Every session in `workspace_id`, most-recently-active first (for the
    /// Recents list). `updated_at` is bumped on each run (see [`Self::run`]),
    /// so a session rises to the top when the user messages it.
    pub async fn list_by_workspace(&self, workspace_id: Uuid) -> StateResult<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, agent_id, title, spec, runenv, created_at, updated_at \
             FROM sessions WHERE workspace_id = ? ORDER BY updated_at DESC",
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

    /// Set a session's title and bump `updated_at`. Backs the manual rename
    /// endpoint; the auto-titler writes its own UPDATE inline (see
    /// [`Self::maybe_generate_title`]).
    pub async fn set_title(&self, id: Uuid, title: &str) -> StateResult<()> {
        let now = Utc::now().to_rfc3339();
        let affected = sqlx::query("UPDATE sessions SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title)
            .bind(now)
            .bind(id.to_string())
            .execute(&self.db)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(StateError::NotFound);
        }
        Ok(())
    }

    /// Whether `id` exists and has no title yet — the gate for auto-titling.
    async fn needs_title(&self, id: Uuid) -> StateResult<bool> {
        let row = sqlx::query("SELECT title FROM sessions WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.db)
            .await?;
        Ok(matches!(row, Some(r) if r.get::<Option<String>, _>("title").is_none()))
    }

    /// If `id` still has no title, spawn a best-effort background task that
    /// summarises `first_text` into one, persists it, and publishes a
    /// [`TitleEvent`] on the session channel. Runs concurrently with the agent
    /// run and never blocks or fails it: on any error the title is simply left
    /// unset and can be regenerated on the next turn.
    async fn maybe_generate_title(&self, id: Uuid, first_text: String) {
        if first_text.trim().is_empty() {
            return;
        }
        match self.needs_title(id).await {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                tracing::warn!(session = %id, "title precheck failed: {e}");
                return;
            }
        }

        let db = self.db.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let title = crate::services::session_title::generate_session_title(&first_text).await;
            let now = Utc::now().to_rfc3339();
            if let Err(e) = sqlx::query("UPDATE sessions SET title = ?, updated_at = ? WHERE id = ?")
                .bind(&title)
                .bind(&now)
                .bind(id.to_string())
                .execute(&db)
                .await
            {
                tracing::warn!(session = %id, "failed to persist session title: {e}");
                return;
            }
            match serde_json::to_string(&TitleEvent { title }) {
                Ok(payload) => {
                    events.publish(&message_channel(id), payload);
                }
                Err(e) => tracing::error!(session = %id, "title event serialize failed: {e}"),
            }
        });
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
        // Drop the channel so any attached WS subscribers wake up with
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

    /// Whether a run is currently in flight for `id`. Used by streaming
    /// handlers to emit an attach-time run-status snapshot.
    pub async fn is_running(&self, id: Uuid) -> bool {
        self.runs.lock().await.contains_key(&id)
    }

    /// Request that any in-flight run for `id` stop at the next safe point.
    /// Returns `true` if a run was found and signaled, `false` if no run was
    /// active. Non-blocking; the spawned task will clean up its own entry.
    pub async fn cancel(&self, id: Uuid) -> bool {
        match self.runs.lock().await.get(&id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Return all messages for `session_id`, ordered by `seq` ascending. Backs
    /// the `GET /sessions/{id}/messages` endpoint; the WS catch-up path uses
    /// [`SessionsState::list_messages_since`] instead.
    pub async fn list_messages(&self, session_id: Uuid) -> StateResult<Vec<(i64, Message)>> {
        self.list_messages_since(session_id, -1).await
    }

    /// Return messages for `session_id` with `seq > since`, ordered ascending.
    /// The WS handler uses this for catch-up before switching to the live
    /// event subscription.
    pub async fn list_messages_since(
        &self,
        session_id: Uuid,
        since: i64,
    ) -> StateResult<Vec<(i64, Message)>> {
        let rows = sqlx::query(
            "SELECT seq, content FROM messages \
             WHERE session_id = ? AND seq > ? ORDER BY seq ASC",
        )
        .bind(session_id.to_string())
        .bind(since)
        .fetch_all(&self.db)
        .await?;
        rows.into_iter()
            .map(|r| -> StateResult<(i64, Message)> {
                let seq: i64 = r.get("seq");
                let content: String = r.get("content");
                let message: Message = serde_json::from_str(&content)?;
                Ok((seq, message))
            })
            .collect()
    }

    /// Trigger an agent run for `id` with `query` as the user turn. Returns
    /// as soon as the run slot is reserved and the background task is
    /// spawned. Each persisted message is also published on the session's
    /// `message/{id}` channel (no-op if no one is subscribed). Cancellation
    /// is requested via [`SessionsState::cancel`].
    ///
    /// Returns [`StateError::AlreadyRunning`] if a run is already in flight
    /// for this session.
    pub async fn run(&self, id: Uuid, query: Vec<Part>) -> StateResult<()> {
        let token = CancellationToken::new();
        {
            let mut runs = self.runs.lock().await;
            if runs.contains_key(&id) {
                return Err(StateError::AlreadyRunning(id));
            }
            runs.insert(id, token.clone());
        }

        let db = self.db.clone();
        let data_root = self.data_root.clone();
        let events = self.events.clone();
        let runs = self.runs.clone();

        // Bump last-activity so the Recents list surfaces this session first.
        // Best-effort: a failure here must never block starting the run.
        let _ = sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(id.to_string())
            .execute(&self.db)
            .await;

        // Auto-title an untitled session from this user turn, concurrent with
        // the run. `query` is moved into the run task below, so extract the
        // text first. Best-effort: `maybe_generate_title` only does a quick
        // "needs a title?" check inline, then spawns the LLM call.
        self.maybe_generate_title(id, first_user_text(&query)).await;

        // Publishing before spawn guarantees a subscriber attached before the
        // POST 202 returns observes `started` before the user-message event.
        self.events.publish(
            &message_channel(id),
            serde_json::to_string(&RunEvent::Started)?,
        );

        tokio::spawn(async move {
            let session_key = id.to_string();
            let dir = data_root.join("sessions").join(&session_key);
            let archive_path = dir.join("sandbox.tar.zst");
            let channel = message_channel(id);

            let result: anyhow::Result<()> = async {
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
                        Sandbox::try_from_archive_with_host_egress(&archive_path).await?;
                    Some(Arc::new(Mutex::new(sandbox)))
                } else {
                    None
                };

                // Mount the unified workspace tree into the guest before the
                // agent runs — local files under `files/` plus the provider
                // mounts as siblings, the browser-WebDAV view served over FUSE
                // at /mnt/workspace. The forward server is held for the whole
                // run; dropping it (at the end of this scope) tears the mount
                // down.
                let _vfs_forward = match &runenv {
                    Some(r) => {
                        let mut sandbox = r.lock().await;
                        let console = sandbox.start().await?;
                        let unified: Arc<dyn crate::vfs::ForwardFs> = Arc::new(
                            crate::state::workspace_fs(&data_root, workspace_id, vfs.clone()),
                        );
                        Some(
                            crate::vfs::sandbox::mount_vfs_in_guest(
                                console,
                                unified,
                                "/mnt/workspace",
                            )
                            .await?,
                        )
                    }
                    None => None,
                };

                let rows = sqlx::query(
                    "SELECT content FROM messages WHERE session_id = ? ORDER BY seq ASC",
                )
                .bind(&session_key)
                .fetch_all(&db)
                .await?;
                let history: Vec<Message> = rows
                    .iter()
                    .map(|r| serde_json::from_str::<Message>(&r.get::<String, _>("content")))
                    .collect::<Result<_, _>>()?;

                // Drive — persist + publish each message. The sandbox archive
                // below must run regardless of how this exits, so capture the
                // drive's Result and propagate it after archiving.
                let drive: anyhow::Result<()> = async {
                    let mut next_seq = history.len() as i64;
                    let mut state = AgentState::new().with_history(history);
                    if let Some(ref r) = runenv {
                        state = state.with_runenv(r.clone());
                    }
                    let mut agent = Agent::try_with_state(spec, state)?;

                    let user_msg = Message::new(Role::User).with_contents(query);
                    sqlx::query(
                        "INSERT INTO messages (session_id, seq, content, created_at) \
                         VALUES (?, ?, ?, ?)",
                    )
                    .bind(&session_key)
                    .bind(next_seq)
                    .bind(serde_json::to_string(&user_msg)?)
                    .bind(Utc::now().to_rfc3339())
                    .execute(&db)
                    .await?;
                    events.publish(
                        &channel,
                        serde_json::to_string(&MessageEvent {
                            seq: next_seq,
                            message: user_msg.clone(),
                        })?,
                    );
                    next_seq += 1;

                    let mut stream = agent.run(user_msg);
                    loop {
                        tokio::select! {
                            biased;
                            _ = token.cancelled() => break,
                            next = stream.next() => {
                                let Some(output) = next else { break };
                                let output = output?;
                                sqlx::query(
                                    "INSERT INTO messages (session_id, seq, content, created_at) \
                                     VALUES (?, ?, ?, ?)",
                                )
                                .bind(&session_key)
                                .bind(next_seq)
                                .bind(serde_json::to_string(&output.message)?)
                                .bind(Utc::now().to_rfc3339())
                                .execute(&db)
                                .await?;
                                events.publish(
                                    &channel,
                                    serde_json::to_string(&MessageEvent {
                                        seq: next_seq,
                                        message: output.message,
                                    })?,
                                );
                                next_seq += 1;
                            }
                        }
                    }
                    Ok(())
                }
                .await;

                let archive_result = if let Some(runenv) = runenv {
                    let mut sandbox = runenv.lock().await;
                    let res: anyhow::Result<()> = async {
                        sandbox.stop().await?;
                        if tokio::fs::try_exists(&archive_path).await? {
                            tokio::fs::remove_file(&archive_path).await?;
                        }
                        sandbox.archive(&archive_path).await?;
                        Ok(())
                    }
                    .await;
                    if let Err(ref e) = res {
                        tracing::error!(session = %id, "sandbox archive failed: {e:#}");
                    }
                    res.map_err(|e| anyhow::anyhow!(e).context("sandbox archive failed"))
                } else {
                    Ok(())
                };

                // Fold archive failure into the run result: if the drive
                // succeeded but archiving failed, surface the archive error.
                drive.and(archive_result)
            }
            .await;

            // Free the run slot BEFORE publishing the terminal event. A client
            // that attaches in between then sees either `is_running == false`
            // or the terminal event on its already-open subscription — never
            // neither. (The reverse order would let a client subscribe after
            // the publish but read `is_running == true`, waiting forever on a
            // run that already ended.)
            runs.lock().await.remove(&id);
            let terminal = match &result {
                Ok(()) => RunEvent::Finished,
                Err(e) => {
                    tracing::error!(session = %id, "run failed: {e:#}");
                    RunEvent::Error {
                        message: format!("{e:#}"),
                    }
                }
            };
            if let Ok(payload) = serde_json::to_string(&terminal) {
                events.publish(&channel, payload);
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh_db() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn is_running_false_for_unknown_id() {
        let pool = fresh_db().await;
        let state = SessionsState::new(pool, std::env::temp_dir(), EventQueue::new());
        let unknown_id = Uuid::new_v4();
        assert!(!state.is_running(unknown_id).await);
    }

    #[tokio::test]
    async fn set_title_missing_session_is_not_found() {
        let pool = fresh_db().await;
        let state = SessionsState::new(pool, std::env::temp_dir(), EventQueue::new());
        let result = state.set_title(Uuid::new_v4(), "x").await;
        assert!(matches!(result, Err(StateError::NotFound)));
    }
}
