mod project;
mod session;

pub use project::*;
pub use session::*;
use url::Url;

use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DbError {
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
}

pub type DbResult<T> = Result<T, DbError>;

pub struct DBStateV2 {
    pool: SqlitePool,
}

impl DBStateV2 {
    /// Build a connection pool from `db_url` (e.g. `"sqlite://./data/app.db"`)
    /// and run any pending migrations.
    pub async fn try_new(url: impl Into<Url>) -> DbResult<Self> {
        let url = url.into();
        let options = url
            .as_str()
            .parse::<SqliteConnectOptions>()
            .map_err(|_| DbError::InvalidData(format!("invalid db url: {url}")))?
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
        Ok(Self { pool })
    }

    pub async fn insert_project(
        &self,
        owner_id: Uuid,
        name: String,
        description: Option<String>,
        slug: Option<String>,
    ) -> DbResult<Project> {
        insert_project(&self.pool, owner_id, name, description, slug).await
    }

    pub async fn get_project(&self, id: Uuid) -> DbResult<Option<Project>> {
        get_project(&self.pool, id).await
    }

    pub async fn get_project_by_slug(&self, slug: &str) -> DbResult<Option<Project>> {
        get_project_by_slug(&self.pool, slug).await
    }

    pub async fn list_projects(&self) -> DbResult<Vec<Project>> {
        list_projects(&self.pool).await
    }

    pub async fn update_project(
        &self,
        id: Uuid,
        name: Option<String>,
        description: Option<Option<String>>,
        slug: Option<String>,
    ) -> DbResult<Project> {
        update_project(&self.pool, id, name, description, slug).await
    }

    pub async fn delete_project(&self, id: Uuid) -> DbResult<bool> {
        delete_project(&self.pool, id).await
    }

    pub async fn insert_session(
        &self,
        project_id: Uuid,
        creator_id: Uuid,
        origin: SessionOrigin,
        agent_type: Option<String>,
        model: Option<String>,
    ) -> DbResult<Session> {
        insert_session(
            &self.pool, project_id, creator_id, origin, agent_type, model,
        )
        .await
    }

    pub async fn get_session(&self, id: Uuid) -> DbResult<Option<Session>> {
        get_session(&self.pool, id).await
    }

    pub async fn list_sessions_in_project(&self, project_id: Uuid) -> DbResult<Vec<Session>> {
        list_sessions_in_project(&self.pool, project_id).await
    }

    pub async fn update_session(
        &self,
        id: Uuid,
        title: Option<Option<String>>,
        share_mode: Option<ShareMode>,
        agent_type: Option<Option<String>>,
        model: Option<Option<String>>,
    ) -> DbResult<Session> {
        update_session(&self.pool, id, title, share_mode, agent_type, model).await
    }

    pub async fn delete_session(&self, id: Uuid) -> DbResult<bool> {
        delete_session(&self.pool, id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn append_message(
        &self,
        session_id: Uuid,
        message_json: String,
        sender_kind: SenderKind,
        sender_name: Option<String>,
        sender_user_id: Option<Uuid>,
        attachments: Vec<String>,
        artifacts: Vec<String>,
        snippet: Option<String>,
    ) -> DbResult<Message> {
        append_message(
            &self.pool,
            session_id,
            message_json,
            sender_kind,
            sender_name,
            sender_user_id,
            attachments,
            artifacts,
            snippet,
        )
        .await
    }

    pub async fn list_messages_by_session(&self, session_id: Uuid) -> DbResult<Vec<Message>> {
        list_messages_by_session(&self.pool, session_id).await
    }

    pub async fn clear_messages_by_session(&self, session_id: Uuid) -> DbResult<u64> {
        clear_messages_by_session(&self.pool, session_id).await
    }
}

// Common utilities

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_uuid(s: String, field: &str) -> DbResult<Uuid> {
    Uuid::parse_str(&s).map_err(|_| DbError::InvalidData(format!("invalid uuid in {field}")))
}

fn parse_ts(s: &str, field: &str) -> DbResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| DbError::InvalidData(format!("invalid timestamp in {field}")))
}

fn map_unique(e: sqlx::Error, field: &str) -> DbError {
    if let sqlx::Error::Database(ref db_err) = e {
        if db_err.message().contains("UNIQUE constraint failed") {
            return DbError::UniqueViolation(field.to_string());
        }
    }
    DbError::Sqlx(e)
}

#[cfg(test)]
pub(super) mod test_helpers {
    use sqlx::SqlitePool;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use uuid::Uuid;

    use super::{insert_project, now_string};

    pub async fn fresh_db() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    pub async fn make_owner(pool: &SqlitePool) -> Uuid {
        let id = Uuid::new_v4();
        let now = now_string();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, is_active, created_at, updated_at) \
             VALUES (?, ?, 'x', 'user', 1, ?, ?)",
        )
        .bind(id.to_string())
        .bind(format!("u-{}", id.simple()))
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    pub async fn make_project(pool: &SqlitePool, owner: Uuid) -> Uuid {
        insert_project(pool, owner, "P".into(), None, None)
            .await
            .unwrap()
            .id
    }
}
