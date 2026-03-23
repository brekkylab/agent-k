use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::models::{Agent, MessageRole, ProviderProfile, Session, SessionMessage};
use crate::repository::{Repository, RepositoryError, RepositoryResult};
use ailoy::{AgentProvider, AgentSpec};

pub struct SqliteRepository {
    pool: SqlitePool,
}

impl SqliteRepository {
    pub async fn new(database_url: &str) -> RepositoryResult<Self> {
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(|_| RepositoryError::InvalidDatabaseUrl(database_url.to_string()))?
            .create_if_missing(true)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        let repository = Self { pool };
        repository.init_schema().await?;
        Ok(repository)
    }

    async fn init_schema(&self) -> RepositoryResult<()> {
        sqlx::query("PRAGMA foreign_keys = ON;")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                spec_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS provider_profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                provider_json TEXT NOT NULL,
                is_default INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                provider_profile_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(agent_id) REFERENCES agents(id) ON DELETE RESTRICT,
                FOREIGN KEY(provider_profile_id) REFERENCES provider_profiles(id) ON DELETE RESTRICT
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_agent_id ON sessions(agent_id);")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_sessions_provider_profile_id ON sessions(provider_profile_id);",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_session_messages_session_id_created_at ON session_messages(session_id, created_at);",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    fn now_string() -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    fn parse_uuid(value: String, field: &str) -> RepositoryResult<Uuid> {
        Uuid::parse_str(&value)
            .map_err(|_| RepositoryError::InvalidData(format!("invalid uuid in field `{field}`")))
    }

    fn parse_timestamp(value: String, field: &str) -> RepositoryResult<DateTime<Utc>> {
        let parsed = DateTime::parse_from_rfc3339(&value).map_err(|_| {
            RepositoryError::InvalidData(format!("invalid timestamp in field `{field}`"))
        })?;
        Ok(parsed.with_timezone(&Utc))
    }

    fn message_role_to_string(role: &MessageRole) -> &'static str {
        match role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        }
    }

    fn message_role_from_string(role: &str) -> RepositoryResult<MessageRole> {
        match role {
            "system" => Ok(MessageRole::System),
            "user" => Ok(MessageRole::User),
            "assistant" => Ok(MessageRole::Assistant),
            "tool" => Ok(MessageRole::Tool),
            _ => Err(RepositoryError::InvalidData(format!(
                "invalid message role `{role}`"
            ))),
        }
    }

    fn row_to_agent(row: &SqliteRow) -> RepositoryResult<Agent> {
        let id = Self::parse_uuid(row.get::<String, _>("id"), "agents.id")?;
        let spec_json = row.get::<String, _>("spec_json");
        let spec = serde_json::from_str::<AgentSpec>(&spec_json)?;
        let created_at =
            Self::parse_timestamp(row.get::<String, _>("created_at"), "agents.created_at")?;
        let updated_at =
            Self::parse_timestamp(row.get::<String, _>("updated_at"), "agents.updated_at")?;

        Ok(Agent {
            id,
            spec,
            created_at,
            updated_at,
        })
    }

    fn row_to_provider_profile(row: &SqliteRow) -> RepositoryResult<ProviderProfile> {
        let id = Self::parse_uuid(row.get::<String, _>("id"), "provider_profiles.id")?;
        let provider_json = row.get::<String, _>("provider_json");
        let provider = serde_json::from_str::<AgentProvider>(&provider_json)?;
        let created_at = Self::parse_timestamp(
            row.get::<String, _>("created_at"),
            "provider_profiles.created_at",
        )?;
        let updated_at = Self::parse_timestamp(
            row.get::<String, _>("updated_at"),
            "provider_profiles.updated_at",
        )?;

        Ok(ProviderProfile {
            id,
            name: row.get::<String, _>("name"),
            provider,
            is_default: row.get::<i64, _>("is_default") != 0,
            created_at,
            updated_at,
        })
    }

    fn row_to_session_without_messages(row: &SqliteRow) -> RepositoryResult<Session> {
        let id = Self::parse_uuid(row.get::<String, _>("id"), "sessions.id")?;
        let agent_id = Self::parse_uuid(row.get::<String, _>("agent_id"), "sessions.agent_id")?;
        let provider_profile_id = Self::parse_uuid(
            row.get::<String, _>("provider_profile_id"),
            "sessions.provider_profile_id",
        )?;
        let created_at =
            Self::parse_timestamp(row.get::<String, _>("created_at"), "sessions.created_at")?;
        let updated_at =
            Self::parse_timestamp(row.get::<String, _>("updated_at"), "sessions.updated_at")?;

        Ok(Session {
            id,
            agent_id,
            provider_profile_id,
            messages: vec![],
            created_at,
            updated_at,
        })
    }

    async fn load_session_messages(
        &self,
        session_id: Uuid,
    ) -> RepositoryResult<Vec<SessionMessage>> {
        let rows = sqlx::query(
            r#"
            SELECT role, content, created_at
            FROM session_messages
            WHERE session_id = ?
            ORDER BY created_at ASC, id ASC;
            "#,
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let role = Self::message_role_from_string(&row.get::<String, _>("role"))?;
            let content = row.get::<String, _>("content");
            let created_at = Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "session_messages.created_at",
            )?;
            messages.push(SessionMessage {
                role,
                content,
                created_at,
            });
        }

        Ok(messages)
    }

    async fn load_session_by_id(&self, id: Uuid) -> RepositoryResult<Option<Session>> {
        let row = sqlx::query(
            r#"
            SELECT id, agent_id, provider_profile_id, created_at, updated_at
            FROM sessions
            WHERE id = ?;
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let mut session = Self::row_to_session_without_messages(&row)?;
        session.messages = self.load_session_messages(session.id).await?;
        Ok(Some(session))
    }
}

#[async_trait]
impl Repository for SqliteRepository {
    async fn create_agent(&self, spec: AgentSpec) -> RepositoryResult<Agent> {
        let now = Self::now_string();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO agents (id, spec_json, created_at, updated_at)
            VALUES (?, ?, ?, ?);
            "#,
        )
        .bind(id.to_string())
        .bind(serde_json::to_string(&spec)?)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(Agent {
            id,
            spec,
            created_at: Self::parse_timestamp(now.clone(), "agents.created_at")?,
            updated_at: Self::parse_timestamp(now, "agents.updated_at")?,
        })
    }

    async fn list_agents(&self) -> RepositoryResult<Vec<Agent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, spec_json, created_at, updated_at
            FROM agents
            ORDER BY created_at ASC, id ASC;
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(Self::row_to_agent).collect()
    }

    async fn get_agent(&self, id: Uuid) -> RepositoryResult<Option<Agent>> {
        let row = sqlx::query(
            r#"
            SELECT id, spec_json, created_at, updated_at
            FROM agents
            WHERE id = ?;
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.as_ref().map(Self::row_to_agent).transpose()
    }

    async fn update_agent(&self, id: Uuid, spec: AgentSpec) -> RepositoryResult<Option<Agent>> {
        let now = Self::now_string();

        let result = sqlx::query(
            r#"
            UPDATE agents
            SET spec_json = ?, updated_at = ?
            WHERE id = ?;
            "#,
        )
        .bind(serde_json::to_string(&spec)?)
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_agent(id).await
    }

    async fn delete_agent(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM agents WHERE id = ?;")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn has_sessions_for_agent(&self, agent_id: Uuid) -> RepositoryResult<bool> {
        let row = sqlx::query("SELECT EXISTS(SELECT 1 FROM sessions WHERE agent_id = ?) AS c;")
            .bind(agent_id.to_string())
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("c") != 0)
    }

    async fn create_provider_profile(
        &self,
        name: String,
        provider: AgentProvider,
        is_default: bool,
    ) -> RepositoryResult<ProviderProfile> {
        let now = Self::now_string();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO provider_profiles
                (id, name, provider_json, is_default, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?);
            "#,
        )
        .bind(id.to_string())
        .bind(name.clone())
        .bind(serde_json::to_string(&provider)?)
        .bind(if is_default { 1_i64 } else { 0_i64 })
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(ProviderProfile {
            id,
            name,
            provider,
            is_default,
            created_at: Self::parse_timestamp(now.clone(), "provider_profiles.created_at")?,
            updated_at: Self::parse_timestamp(now, "provider_profiles.updated_at")?,
        })
    }

    async fn list_provider_profiles(&self) -> RepositoryResult<Vec<ProviderProfile>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, provider_json, is_default, created_at, updated_at
            FROM provider_profiles
            ORDER BY created_at ASC, id ASC;
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(Self::row_to_provider_profile).collect()
    }

    async fn get_provider_profile(&self, id: Uuid) -> RepositoryResult<Option<ProviderProfile>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, provider_json, is_default, created_at, updated_at
            FROM provider_profiles
            WHERE id = ?;
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.as_ref().map(Self::row_to_provider_profile).transpose()
    }

    async fn update_provider_profile(
        &self,
        id: Uuid,
        name: String,
        provider: AgentProvider,
        is_default: bool,
    ) -> RepositoryResult<Option<ProviderProfile>> {
        let now = Self::now_string();

        let result = sqlx::query(
            r#"
            UPDATE provider_profiles
            SET name = ?, provider_json = ?, is_default = ?, updated_at = ?
            WHERE id = ?;
            "#,
        )
        .bind(name)
        .bind(serde_json::to_string(&provider)?)
        .bind(if is_default { 1_i64 } else { 0_i64 })
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_provider_profile(id).await
    }

    async fn delete_provider_profile(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM provider_profiles WHERE id = ?;")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn has_sessions_for_provider_profile(
        &self,
        provider_profile_id: Uuid,
    ) -> RepositoryResult<bool> {
        let row = sqlx::query(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE provider_profile_id = ?) AS c;",
        )
        .bind(provider_profile_id.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("c") != 0)
    }

    async fn upsert_provider_profile_by_name(
        &self,
        name: &str,
        provider: AgentProvider,
        is_default: bool,
    ) -> RepositoryResult<ProviderProfile> {
        let existing = sqlx::query(
            r#"
            SELECT id
            FROM provider_profiles
            WHERE name = ?;
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        let now = Self::now_string();
        let provider_json = serde_json::to_string(&provider)?;

        if let Some(row) = existing {
            let id = Self::parse_uuid(row.get::<String, _>("id"), "provider_profiles.id")?;
            sqlx::query(
                r#"
                UPDATE provider_profiles
                SET provider_json = ?, is_default = ?, updated_at = ?
                WHERE id = ?;
                "#,
            )
            .bind(provider_json)
            .bind(if is_default { 1_i64 } else { 0_i64 })
            .bind(now)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

            return self.get_provider_profile(id).await?.ok_or_else(|| {
                RepositoryError::InvalidData("upsert updated row not found".to_string())
            });
        }

        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO provider_profiles
                (id, name, provider_json, is_default, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?);
            "#,
        )
        .bind(id.to_string())
        .bind(name)
        .bind(provider_json)
        .bind(if is_default { 1_i64 } else { 0_i64 })
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.get_provider_profile(id).await?.ok_or_else(|| {
            RepositoryError::InvalidData("upsert inserted row not found".to_string())
        })
    }

    async fn create_session(
        &self,
        agent_id: Uuid,
        provider_profile_id: Uuid,
    ) -> RepositoryResult<Session> {
        let now = Self::now_string();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO sessions (id, agent_id, provider_profile_id, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?);
            "#,
        )
        .bind(id.to_string())
        .bind(agent_id.to_string())
        .bind(provider_profile_id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.load_session_by_id(id)
            .await?
            .ok_or_else(|| RepositoryError::InvalidData("created session not found".to_string()))
    }

    async fn list_sessions(&self, agent_id: Option<Uuid>) -> RepositoryResult<Vec<Session>> {
        let rows = if let Some(agent_id) = agent_id {
            sqlx::query(
                r#"
                SELECT id, agent_id, provider_profile_id, created_at, updated_at
                FROM sessions
                WHERE agent_id = ?
                ORDER BY created_at ASC, id ASC;
                "#,
            )
            .bind(agent_id.to_string())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT id, agent_id, provider_profile_id, created_at, updated_at
                FROM sessions
                ORDER BY created_at ASC, id ASC;
                "#,
            )
            .fetch_all(&self.pool)
            .await?
        };

        let mut sessions = Vec::with_capacity(rows.len());
        for row in rows {
            let mut session = Self::row_to_session_without_messages(&row)?;
            session.messages = self.load_session_messages(session.id).await?;
            sessions.push(session);
        }

        Ok(sessions)
    }

    async fn get_session(&self, id: Uuid) -> RepositoryResult<Option<Session>> {
        self.load_session_by_id(id).await
    }

    async fn delete_session(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?;")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn add_session_message(
        &self,
        session_id: Uuid,
        role: MessageRole,
        content: String,
    ) -> RepositoryResult<Option<Session>> {
        let mut tx = self.pool.begin().await?;

        let exists = sqlx::query("SELECT 1 FROM sessions WHERE id = ?;")
            .bind(session_id.to_string())
            .fetch_optional(tx.as_mut())
            .await?
            .is_some();

        if !exists {
            tx.rollback().await?;
            return Ok(None);
        }

        let now = Self::now_string();
        sqlx::query(
            r#"
            INSERT INTO session_messages (id, session_id, role, content, created_at)
            VALUES (?, ?, ?, ?, ?);
            "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(session_id.to_string())
        .bind(Self::message_role_to_string(&role))
        .bind(content)
        .bind(&now)
        .execute(tx.as_mut())
        .await?;

        sqlx::query(
            r#"
            UPDATE sessions
            SET updated_at = ?
            WHERE id = ?;
            "#,
        )
        .bind(now)
        .bind(session_id.to_string())
        .execute(tx.as_mut())
        .await?;

        tx.commit().await?;
        self.load_session_by_id(session_id).await
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use url::Url;

    use super::SqliteRepository;
    use crate::models::MessageRole;
    use crate::repository::Repository;
    use ailoy::{AgentProvider, AgentSpec, LangModelAPISchema, LangModelProvider};

    #[actix_web::test]
    async fn sqlite_persists_data_between_repository_instances() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("app.db");
        let db_url = format!("sqlite://{}", db_path.display());

        let repository = SqliteRepository::new(&db_url)
            .await
            .expect("sqlite repository should be created");

        let agent = repository
            .create_agent(AgentSpec::new("gpt-4.1"))
            .await
            .expect("agent should be created");

        let provider_profile = repository
            .create_provider_profile(
                "openai-default".to_string(),
                AgentProvider {
                    lm: LangModelProvider::API {
                        schema: LangModelAPISchema::ChatCompletion,
                        url: Url::parse("https://api.openai.com/v1/chat/completions")
                            .expect("url should parse"),
                        api_key: Some("secret".to_string()),
                    },
                    tools: vec![],
                },
                true,
            )
            .await
            .expect("provider profile should be created");

        let session = repository
            .create_session(agent.id, provider_profile.id)
            .await
            .expect("session should be created");

        repository
            .add_session_message(session.id, MessageRole::User, "hello".to_string())
            .await
            .expect("message should be inserted")
            .expect("session should exist");

        drop(repository);

        let restarted_repository = SqliteRepository::new(&db_url)
            .await
            .expect("sqlite repository should reopen");

        let agents = restarted_repository
            .list_agents()
            .await
            .expect("agents should be loaded");
        assert_eq!(agents.len(), 1);

        let sessions = restarted_repository
            .list_sessions(None)
            .await
            .expect("sessions should be loaded");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].messages.len(), 1);
    }
}
