mod sqlite;

pub use sqlite::SqliteRepository;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_DB_PATH: &str = "sqlite://./data/agent-k.db";

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("invalid database URL: {0}")]
    InvalidDatabaseUrl(String),

    #[error("invalid data: {0}")]
    InvalidData(String),
}

pub type RepositoryResult<T> = Result<T, RepositoryError>;

#[derive(Debug, Clone)]
pub struct DbSession {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
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
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    let repo = SqliteRepository::new(pool);
    repo.migrate().await?;
    Ok(Arc::new(repo))
}
