use ailoy::message::Message;
use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::repository::{DbSession, RepositoryError, RepositoryResult};

pub struct SqliteRepository {
    pool: SqlitePool,
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> RepositoryResult<()> {
        sqlx::query("PRAGMA foreign_keys = ON;")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA synchronous = NORMAL;")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_messages (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                message_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_session_messages_session_seq \
             ON session_messages(session_id, seq);",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    fn now_string() -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    fn parse_uuid(s: String, field: &str) -> RepositoryResult<Uuid> {
        Uuid::parse_str(&s)
            .map_err(|_| RepositoryError::InvalidData(format!("invalid uuid in {field}")))
    }

    fn parse_timestamp(s: String, field: &str) -> RepositoryResult<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| RepositoryError::InvalidData(format!("invalid timestamp in {field}")))
    }

    pub async fn create_session(&self, id: Uuid) -> RepositoryResult<DbSession> {
        let now = Self::now_string();
        sqlx::query("INSERT INTO sessions (id, created_at, updated_at) VALUES (?, ?, ?);")
            .bind(id.to_string())
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await?;

        Ok(DbSession {
            id,
            created_at: Self::parse_timestamp(now.clone(), "sessions.created_at")?,
            updated_at: Self::parse_timestamp(now, "sessions.updated_at")?,
        })
    }

    pub async fn get_session(&self, id: Uuid) -> RepositoryResult<Option<DbSession>> {
        let row = sqlx::query("SELECT id, created_at, updated_at FROM sessions WHERE id = ?;")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(DbSession {
            id: Self::parse_uuid(row.get::<String, _>("id"), "sessions.id")?,
            created_at: Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "sessions.created_at",
            )?,
            updated_at: Self::parse_timestamp(
                row.get::<String, _>("updated_at"),
                "sessions.updated_at",
            )?,
        }))
    }

    pub async fn delete_session(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?;")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn append_messages(
        &self,
        session_id: Uuid,
        messages: &[Message],
    ) -> RepositoryResult<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let now = Self::now_string();
        let sid = session_id.to_string();

        for msg in messages {
            let msg_json = serde_json::to_string(msg)?;
            sqlx::query(
                "INSERT INTO session_messages (session_id, message_json, created_at) \
                 VALUES (?, ?, ?);",
            )
            .bind(&sid)
            .bind(&msg_json)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        }

        sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?;")
            .bind(&now)
            .bind(&sid)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn get_messages(&self, session_id: Uuid) -> RepositoryResult<Vec<Message>> {
        let rows = sqlx::query(
            "SELECT message_json FROM session_messages \
             WHERE session_id = ? ORDER BY seq ASC;",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let json = row.get::<String, _>("message_json");
                serde_json::from_str::<Message>(&json).map_err(RepositoryError::Serialization)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ailoy::message::{Message, Part, Role};
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use tempfile::tempdir;
    use uuid::Uuid;

    use super::SqliteRepository;

    async fn make_repo(db_url: &str) -> SqliteRepository {
        let options = db_url
            .parse::<SqliteConnectOptions>()
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap();

        let repo = SqliteRepository::new(pool);
        repo.migrate().await.unwrap();
        repo
    }

    #[tokio::test]
    async fn session_and_messages_survive_repository_restart() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let session_id = Uuid::new_v4();

        // First "server instance": create session and write messages.
        {
            let repo = make_repo(&db_url).await;
            repo.create_session(session_id).await.unwrap();

            let msgs = vec![
                Message::new(Role::User).with_contents([Part::text("What is 1+1?")]),
                Message::new(Role::Assistant).with_contents([Part::text("1+1 equals 2.")]),
            ];
            repo.append_messages(session_id, &msgs).await.unwrap();

            let fetched = repo.get_messages(session_id).await.unwrap();
            assert_eq!(fetched.len(), 2);
        }

        // Second "server instance": open same DB and verify data is intact.
        {
            let repo = make_repo(&db_url).await;

            let session = repo.get_session(session_id).await.unwrap();
            assert!(session.is_some(), "session must survive restart");

            let fetched = repo.get_messages(session_id).await.unwrap();
            assert_eq!(fetched.len(), 2);
            assert!(matches!(fetched[0].role, Role::User));
            assert!(matches!(fetched[1].role, Role::Assistant));

            let user_text = fetched[0]
                .contents
                .iter()
                .find_map(|p| p.as_text())
                .unwrap_or("");
            assert_eq!(user_text, "What is 1+1?");
        }
    }

    #[tokio::test]
    async fn delete_session_cascades_messages() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let repo = make_repo(&db_url).await;
        let session_id = Uuid::new_v4();

        repo.create_session(session_id).await.unwrap();
        repo.append_messages(
            session_id,
            &[Message::new(Role::User).with_contents([Part::text("hello")])],
        )
        .await
        .unwrap();

        assert_eq!(repo.get_messages(session_id).await.unwrap().len(), 1);

        let deleted = repo.delete_session(session_id).await.unwrap();
        assert!(deleted);

        // Messages must be cascade-deleted.
        assert_eq!(repo.get_messages(session_id).await.unwrap().len(), 0);

        // Session itself must be gone.
        assert!(repo.get_session(session_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_messages_preserves_insertion_order() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let repo = make_repo(&db_url).await;
        let sid = Uuid::new_v4();
        repo.create_session(sid).await.unwrap();

        let batch1 = vec![
            Message::new(Role::User).with_contents([Part::text("turn1 user")]),
            Message::new(Role::Assistant).with_contents([Part::text("turn1 assistant")]),
        ];
        repo.append_messages(sid, &batch1).await.unwrap();

        let batch2 = vec![
            Message::new(Role::User).with_contents([Part::text("turn2 user")]),
            Message::new(Role::Assistant).with_contents([Part::text("turn2 assistant")]),
        ];
        repo.append_messages(sid, &batch2).await.unwrap();

        let all = repo.get_messages(sid).await.unwrap();
        assert_eq!(all.len(), 4);

        let texts: Vec<&str> = all
            .iter()
            .flat_map(|m| m.contents.iter().filter_map(|p| p.as_text()))
            .collect();

        assert_eq!(
            texts,
            [
                "turn1 user",
                "turn1 assistant",
                "turn2 user",
                "turn2 assistant"
            ]
        );
    }
}
