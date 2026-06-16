use std::{collections::HashSet, path::PathBuf, sync::Arc};

use ailoy::{
    agent::{Agent, AgentSpec, AgentState},
    message::{Message, MessageOutput, Part, Role},
    runenv::{Machine as _, Sandbox},
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use tokio::sync::{Mutex, mpsc, oneshot};
use uuid::Uuid;

use super::{StateError, StateResult, parse_ts, parse_uuid};

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

    /// Tracks sessions currently inside [`SessionsState::run`] to prevent
    /// two concurrent runs against the same session.
    running: Arc<Mutex<HashSet<Uuid>>>,
}

impl SessionsState {
    pub fn new(db: SqlitePool, data_root: PathBuf) -> Self {
        Self {
            db,
            data_root,
            running: Arc::new(Mutex::new(HashSet::new())),
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
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        let dir = self.data_root.join(id.to_string());
        if tokio::fs::try_exists(&dir).await? {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        Ok(existing)
    }

    /// Restore the agent from the persisted spec + sandbox snapshot, replay
    /// history from the `messages` table, and run a single turn with `query`.
    /// Each emitted message is inserted into the `messages` table as it
    /// arrives, and the sandbox is re-archived on the way out — whether the
    /// run finishes normally, errors, or is canceled.
    ///
    /// Returns an [`mpsc::Receiver`] that yields each [`MessageOutput`] (or an
    /// error) and a [`oneshot::Sender`] that aborts the run when signaled.
    pub fn run(
        &self,
        id: Uuid,
        query: Vec<Part>,
    ) -> (
        mpsc::Receiver<anyhow::Result<MessageOutput>>,
        oneshot::Sender<()>,
    ) {
        let (tx, rx) = mpsc::channel::<anyhow::Result<MessageOutput>>(32);
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();

        let db = self.db.clone();
        let data_root = self.data_root.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            // Reserve the run slot before doing any work. If another run is
            // already in flight we must NOT remove the id later.
            if !running.lock().await.insert(id) {
                let _ = tx
                    .send(Err(anyhow::anyhow!("session {id} is already running")))
                    .await;
                return;
            }

            let session_key = id.to_string();
            let dir = data_root.join(&session_key);
            let archive_path = dir.join("sandbox.tar.zst");

            // Setup phase — load spec from the DB row, history from messages,
            // and the sandbox archive if one exists. Sessions without a
            // sandbox skip all sandbox plumbing below.
            let setup: anyhow::Result<(AgentSpec, Option<Arc<Mutex<Sandbox>>>, Vec<Message>)> =
                async {
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
                    Ok((spec, runenv, history))
                }
                .await;

            let (spec, runenv, history) = match setup {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    running.lock().await.remove(&id);
                    return;
                }
            };

            // Drive phase — persist each message eagerly. Any error here
            // exits the loop but we still fall through to archive below.
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
                next_seq += 1;

                let mut stream = agent.run(user_msg);
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut cancel_rx => break,
                        next = stream.next() => {
                            let Some(output) = next else { break; };
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
                            next_seq += 1;
                            if tx.send(Ok(output)).await.is_err() {
                                // Receiver dropped — treat as cancellation.
                                break;
                            }
                        }
                    }
                }
                Ok(())
            }
            .await;

            if let Err(e) = drive {
                let _ = tx.send(Err(e)).await;
            }

            // Best-effort sandbox archive: runs whether the drive phase
            // finished normally, errored, or was canceled. Skipped entirely
            // for sandbox-less sessions.
            if let Some(runenv) = runenv {
                let archive: anyhow::Result<()> = async {
                    let mut sandbox = runenv.lock().await;
                    sandbox.stop().await?;
                    if tokio::fs::try_exists(&archive_path).await? {
                        tokio::fs::remove_file(&archive_path).await?;
                    }
                    sandbox.archive(&archive_path).await?;
                    Ok(())
                }
                .await;
                if let Err(e) = archive {
                    let _ = tx
                        .send(Err(anyhow::anyhow!("sandbox archive failed: {e:#}")))
                        .await;
                }
            }

            running.lock().await.remove(&id);
        });

        (rx, cancel_tx)
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::state::{Project, ProjectsState};

//     /// End-to-end smoke test: build a coworker agent from agent-k, snapshot it
//     /// into `<workspace>/data` via [`SessionsState::insert`], then drive a
//     /// single [`SessionsState::run`] turn and persist its outputs.
//     ///
//     /// Requires microsandbox + a valid `ANTHROPIC_API_KEY` in the environment,
//     /// so it's gated behind `--ignored`.
//     #[tokio::test]
//     #[ignore = "requires microsandbox runtime and ANTHROPIC_API_KEY"]
//     async fn run_coworker_session_end_to_end() {
//         dotenvy::dotenv().ok();

//         let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
//             .parent()
//             .expect("backend_v2 has a parent")
//             .to_path_buf();
//         let data_root = workspace_root.join("data");
//         tokio::fs::create_dir_all(&data_root).await.unwrap();

//         let input_dir = tempfile::tempdir().unwrap();
//         let shared_dir = tempfile::tempdir().unwrap();
//         let artifacts_dir = tempfile::tempdir().unwrap();

//         let pool = SqlitePool::connect(":memory:").await.unwrap();
//         sqlx::migrate!("./migrations").run(&pool).await.unwrap();

//         let projects = ProjectsState::new(pool.clone());
//         let sessions = SessionsState::new(pool, data_root);

//         let project = Project::new("Coworker test".into());
//         projects.upsert(project.clone()).await.unwrap();

//         let session = Session::new(project.id).with_title("smoke");
//         let session_id = session.id;

//         let spec = agent_k::agents::get_coworker_agent_spec(
//             "test-coworker",
//             "anthropic/claude-opus-4-7",
//             false,
//         );
//         let runenv = agent_k::agents::get_coworker_agent_runenv(
//             input_dir.path(),
//             shared_dir.path(),
//             artifacts_dir.path(),
//         )
//         .await
//         .unwrap();

//         sessions.insert(session, spec, runenv).await.unwrap();

//         let mut stream = sessions.run(
//             session_id,
//             vec![Part::text("Say hi in one short sentence.")],
//         );
//         let mut emitted = 0usize;
//         while let Some(item) = stream.next().await {
//             let output = item.unwrap();
//             emitted += 1;
//             eprintln!(
//                 "[{emitted}] role={:?} finish={:?}",
//                 output.message.role, output.finish_reason
//             );
//         }
//         assert!(emitted > 0, "agent produced no messages");
//     }
// }
