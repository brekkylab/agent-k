mod sqlite;

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
pub use sqlite::SqliteRepository;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::Role;

const DEFAULT_DB_PATH: &str = "sqlite://./data/agent-k.db";

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("invalid database URL: {0}")]
    InvalidDatabaseUrl(String),

    #[error("invalid data: {0}")]
    InvalidData(String),

    #[error("unique constraint violation on {0}")]
    UniqueViolation(String),
}

pub type RepositoryResult<T> = Result<T, RepositoryError>;

#[derive(Debug, Clone)]
pub struct DbSession {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DbUser {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub display_name: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct NewUser {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub display_name: Option<String>,
    pub is_active: bool,
}

pub struct UpdateUser {
    pub display_name: Option<String>,
    pub password_hash: Option<String>,
    pub role: Option<Role>,
    pub is_active: Option<bool>,
}

pub type AppRepository = Arc<SqliteRepository>;

pub async fn create_repository_from_env() -> RepositoryResult<AppRepository> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_PATH.to_string());
    if db_url == DEFAULT_DB_PATH {
        std::fs::create_dir_all("./data")
            .map_err(|e| RepositoryError::InvalidData(format!("failed to create data dir: {e}")))?;
    }
    create_repository(&db_url).await
}

pub async fn create_repository(db_url: &str) -> RepositoryResult<AppRepository> {
    let options = db_url
        .parse::<SqliteConnectOptions>()
        .map_err(|_| RepositoryError::InvalidDatabaseUrl(db_url.to_string()))?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .synchronous(SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(Arc::new(SqliteRepository::new(pool)))
}
