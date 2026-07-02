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
}
