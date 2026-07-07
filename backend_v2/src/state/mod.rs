use std::{path::PathBuf, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;
use uuid::Uuid;

use crate::{auth::JwtConfig, event::EventQueue};

mod agent;
mod session;
mod user;
mod workspace;

pub use agent::*;
pub use session::*;
pub use user::*;
pub use workspace::*;

pub(crate) fn parse_uuid(raw: String, field: &str) -> StateResult<Uuid> {
    Uuid::parse_str(&raw).map_err(|e| StateError::InvalidData(format!("{field}: {e}")))
}

pub(crate) fn parse_ts(raw: &str, field: &str) -> StateResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| StateError::InvalidData(format!("{field}: {e}")))
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("migration: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("invalid data: {0}")]
    InvalidData(String),

    #[error("not found")]
    NotFound,

    #[error("unique constraint violation on {0}")]
    UniqueViolation(String),

    #[error("session {0} is already running")]
    AlreadyRunning(Uuid),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("sandbox: {0}")]
    Sandbox(String),
}

pub type StateResult<T> = Result<T, StateError>;

pub struct AppState {
    pub workspaces: WorkspacesState,
    pub agents: AgentsState,
    pub sessions: SessionsState,
    pub users: UsersState,
    pub events: EventQueue,
    pub jwt: JwtConfig,
}

impl AppState {
    pub async fn new(db_url: &str, data_root: PathBuf, jwt: JwtConfig) -> StateResult<Self> {
        let options = db_url
            .parse::<SqliteConnectOptions>()
            .map_err(|e| StateError::InvalidData(format!("DATABASE_URL: {e}")))?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5))
            .synchronous(SqliteSynchronous::Normal);

        let db = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        sqlx::migrate!("./migrations").run(&db).await?;

        let events = EventQueue::new();

        Ok(Self {
            workspaces: WorkspacesState::new(db.clone(), data_root.clone()),
            agents: AgentsState::new(db.clone()),
            sessions: SessionsState::new(db.clone(), data_root, events.clone()),
            users: UsersState::new(db),
            events,
            jwt,
        })
    }

    // --- Deletion choreography -------------------------------------------
    //
    // The database cascades `user → workspace → agent → session → messages`
    // on delete, but a raw row cascade only drops rows: it never cancels an
    // in-flight run, removes a session's on-disk dir, closes its event
    // channel, or deletes a workspace's files. Each `delete_*` below is the
    // single definition of "what tearing down this entity means" — collect the
    // descendant session ids *before* the cascade wipes their rows, delete the
    // row(s), then best-effort sweep the artifacts the FK can't reach. Every
    // caller (HTTP routes, etc.) goes through these instead of touching the
    // per-entity `remove` directly, so no delete path can forget the teardown.

    /// Delete a session and its artifacts (leaf of the cascade).
    pub async fn delete_session(&self, id: Uuid) -> StateResult<Session> {
        self.sessions.remove(id).await
    }

    /// Delete an agent and sweep its (cascade-dropped) sessions' artifacts.
    pub async fn delete_agent(&self, id: Uuid) -> StateResult<Agent> {
        let session_ids = self.sessions.ids_by_agent(id).await?;
        let agent = self.agents.remove(id).await?;
        self.discard_session_artifacts(session_ids).await;
        Ok(agent)
    }

    /// Delete a workspace, its files, and its sessions' artifacts.
    pub async fn delete_workspace(&self, id: Uuid) -> StateResult<Workspace> {
        let session_ids = self.sessions.ids_by_workspace(id).await?;
        let workspace = self.workspaces.remove(id).await?;
        self.discard_session_artifacts(session_ids).await;
        Ok(workspace)
    }

    /// Delete a user and every descendant artifact; `false` if no row existed.
    /// Rows drop atomically first (retryable on failure), disk cleanup after
    /// (a failure only leaks disk). Default workspace id == user id.
    pub async fn delete_user(&self, id: Uuid) -> StateResult<bool> {
        let session_ids = self.sessions.ids_by_workspace(id).await?;
        if !self.users.delete_with_default_workspace(id).await? {
            return Ok(false);
        }
        self.discard_session_artifacts(session_ids).await;
        if let Err(e) = self.workspaces.remove_files(id).await {
            tracing::error!(user_id = %id, "failed to remove workspace files: {e}");
        }
        Ok(true)
    }

    /// Best-effort teardown of run/dir/channel artifacts for sessions whose
    /// rows are already gone. Never fails; logs and skips on error.
    async fn discard_session_artifacts(&self, ids: Vec<Uuid>) {
        for id in ids {
            self.sessions.discard_artifacts(id).await;
        }
    }
}
