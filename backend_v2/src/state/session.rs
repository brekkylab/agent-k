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
use crate::event::{EventQueue, MessageEvent, message_channel};

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,

    pub project_id: Uuid,

    pub title: Option<String>,

    pub spec: AgentSpec,

    /// Whether this session has a backbone run environment
    pub runenv: bool,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn new(project_id: Uuid, spec: AgentSpec) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            project_id,
            title: None,
            spec,
            runenv: false,
            created_at: now,
            updated_at: now,
        }
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
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "sessions.id")?,
            project_id: parse_uuid(row.get::<String, _>("project_id"), "sessions.project_id")?,
            title: row.get("title"),
            spec,
            runenv: row.get("runenv"),
            created_at: parse_ts(&row.get::<String, _>("created_at"), "sessions.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "sessions.updated_at")?,
        })
    }
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

    pub async fn list(&self) -> StateResult<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT id, project_id, title, spec, runenv, created_at, updated_at \
             FROM sessions ORDER BY created_at ASC",
        )
        .fetch_all(&self.db)
        .await?;
        rows.iter().map(Session::from_sqlite_row).collect()
    }

    pub async fn get(&self, id: Uuid) -> StateResult<Option<Session>> {
        let row = sqlx::query(
            "SELECT id, project_id, title, spec, runenv, created_at, updated_at \
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
            "INSERT INTO sessions (id, project_id, title, spec, runenv, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item.id.to_string())
        .bind(item.project_id.to_string())
        .bind(&item.title)
        .bind(serde_json::to_string(&item.spec)?)
        .bind(item.runenv)
        .bind(item.created_at.to_rfc3339())
        .bind(item.updated_at.to_rfc3339())
        .execute(&self.db)
        .await?;

        if let Some(mut runenv) = runenv {
            let dir = self.data_root.join(item.id.to_string());
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
        let dir = self.data_root.join(id.to_string());
        if tokio::fs::try_exists(&dir).await? {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        // Drop the channel so any attached WS subscribers wake up with
        // RecvError::Closed instead of waiting on a session that no longer
        // exists.
        self.events.remove_channel(&message_channel(id));
        Ok(existing)
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

        tokio::spawn(async move {
            let session_key = id.to_string();
            let dir = data_root.join(&session_key);
            let archive_path = dir.join("sandbox.tar.zst");
            let channel = message_channel(id);

            let result: anyhow::Result<()> = async {
                // Setup — spec, history, sandbox.
                let row = sqlx::query("SELECT spec, runenv FROM sessions WHERE id = ?")
                    .bind(&session_key)
                    .fetch_optional(&db)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("session {id} not found"))?;
                let spec: AgentSpec = serde_json::from_str(&row.get::<String, _>("spec"))?;
                let has_runenv: bool = row.get("runenv");

                let runenv = if has_runenv {
                    if !tokio::fs::try_exists(&archive_path).await? {
                        anyhow::bail!(
                            "session {id} marked as having a runenv but archive is missing at {}",
                            archive_path.display()
                        );
                    }
                    Some(Arc::new(Mutex::new(
                        Sandbox::try_from_archive(&archive_path).await?,
                    )))
                } else {
                    None
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

            if let Err(e) = result {
                tracing::error!(session = %id, "run failed: {e:#}");
            }
            runs.lock().await.remove(&id);
        });

        Ok(())
    }
}
