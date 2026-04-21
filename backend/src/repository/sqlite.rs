use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use ailoy::{AgentProvider, AgentSpec};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::models::{
    Agent, MessageRole, ProviderProfile, Session, SessionMessage, SessionToolCall, Source,
    SourceType, Speedwagon, SpeedwagonIndexStatus,
};
use crate::repository::{Repository, RepositoryError, RepositoryResult, normalize_spec};

pub struct SqliteRepository {
    pool: SqlitePool,
}

impl SqliteRepository {
    pub async fn new(database_url: &str) -> RepositoryResult<Self> {
        let options = SqliteConnectOptions::from_str(database_url)
            .map_err(|_| RepositoryError::InvalidDatabaseUrl(database_url.to_string()))?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));

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
        // WAL mode + NORMAL sync: safe for WAL, skips redundant fsync per commit
        sqlx::query("PRAGMA synchronous = NORMAL;")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                spec_json TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Belt-and-suspenders: CREATE TABLE IF NOT EXISTS silently ignores the inline
        // UNIQUE on existing DBs. Add an explicit unique index so pre-existing tables
        // also get the constraint. Fails with SQLITE_CONSTRAINT if duplicate spec_json
        // rows already exist — in that case delete backend/data/app.db (dev stage).
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_agents_spec_json ON agents(spec_json);",
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
                title TEXT,
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

        // --- Sources ---
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sources (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source_type TEXT NOT NULL DEFAULT 'local_file',
                file_path TEXT,
                size INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sources_created_at ON sources(created_at);")
            .execute(&self.pool)
            .await?;

        // --- Speedwagons ---
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS speedwagons (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                instruction TEXT,
                lm TEXT,
                index_dir TEXT,
                corpus_dir TEXT,
                index_status TEXT NOT NULL DEFAULT 'not_indexed',
                index_error TEXT,
                index_started_at TEXT,
                indexed_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS speedwagon_sources (
                speedwagon_id TEXT NOT NULL,
                source_id TEXT NOT NULL,
                PRIMARY KEY (speedwagon_id, source_id),
                FOREIGN KEY(speedwagon_id) REFERENCES speedwagons(id) ON DELETE CASCADE,
                FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_speedwagon_sources_source_id ON speedwagon_sources(source_id);",
        )
        .execute(&self.pool)
        .await?;

        // Idempotent: SQLite errors on duplicate column; ignore the error.
        let _ = sqlx::query("ALTER TABLE speedwagons ADD COLUMN provider_profile_id TEXT;")
            .execute(&self.pool)
            .await;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_speedwagons (
                session_id TEXT NOT NULL,
                speedwagon_id TEXT NOT NULL,
                PRIMARY KEY (session_id, speedwagon_id),
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY(speedwagon_id) REFERENCES speedwagons(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_sources (
                session_id TEXT NOT NULL,
                source_id TEXT NOT NULL,
                PRIMARY KEY (session_id, source_id),
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY(source_id) REFERENCES sources(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // --- Session Tool Calls ---
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_tool_calls (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                tool_args TEXT,
                tool_result TEXT,
                duration_ms INTEGER,
                created_at TEXT NOT NULL,
                FOREIGN KEY(message_id) REFERENCES session_messages(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_session_tool_calls_message_id ON session_tool_calls(message_id);",
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

    fn parse_optional_uuid(value: Option<String>, field: &str) -> RepositoryResult<Option<Uuid>> {
        match value {
            None => Ok(None),
            Some(s) => Self::parse_uuid(s, field).map(Some),
        }
    }

    fn parse_optional_timestamp(
        value: Option<String>,
        field: &str,
    ) -> RepositoryResult<Option<DateTime<Utc>>> {
        match value {
            None => Ok(None),
            Some(s) => Self::parse_timestamp(s, field).map(Some),
        }
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

    fn index_status_to_string(status: &SpeedwagonIndexStatus) -> &'static str {
        match status {
            SpeedwagonIndexStatus::NotIndexed => "not_indexed",
            SpeedwagonIndexStatus::Indexing => "indexing",
            SpeedwagonIndexStatus::Indexed => "indexed",
            SpeedwagonIndexStatus::Error => "error",
        }
    }

    fn index_status_from_string(s: &str) -> RepositoryResult<SpeedwagonIndexStatus> {
        match s {
            "not_indexed" => Ok(SpeedwagonIndexStatus::NotIndexed),
            "indexing" => Ok(SpeedwagonIndexStatus::Indexing),
            "indexed" => Ok(SpeedwagonIndexStatus::Indexed),
            "error" => Ok(SpeedwagonIndexStatus::Error),
            _ => Err(RepositoryError::InvalidData(format!(
                "invalid index status `{s}`"
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

    fn source_type_from_string(s: &str) -> RepositoryResult<SourceType> {
        match s {
            "local_file" => Ok(SourceType::LocalFile),
            _ => Err(RepositoryError::InvalidData(format!(
                "invalid source type `{s}`"
            ))),
        }
    }

    fn row_to_source(row: &SqliteRow) -> RepositoryResult<Source> {
        let id = Self::parse_uuid(row.get::<String, _>("id"), "sources.id")?;
        let source_type = Self::source_type_from_string(&row.get::<String, _>("source_type"))?;
        let created_at =
            Self::parse_timestamp(row.get::<String, _>("created_at"), "sources.created_at")?;
        let updated_at =
            Self::parse_timestamp(row.get::<String, _>("updated_at"), "sources.updated_at")?;

        Ok(Source {
            id,
            name: row.get::<String, _>("name"),
            source_type,
            file_path: row.get::<Option<String>, _>("file_path"),
            size: row.get::<i64, _>("size"),
            created_at,
            updated_at,
        })
    }

    fn row_to_speedwagon_without_sources(row: &SqliteRow) -> RepositoryResult<Speedwagon> {
        let id = Self::parse_uuid(row.get::<String, _>("id"), "speedwagons.id")?;
        let index_status = Self::index_status_from_string(&row.get::<String, _>("index_status"))?;
        let created_at =
            Self::parse_timestamp(row.get::<String, _>("created_at"), "speedwagons.created_at")?;
        let updated_at =
            Self::parse_timestamp(row.get::<String, _>("updated_at"), "speedwagons.updated_at")?;
        let index_started_at = Self::parse_optional_timestamp(
            row.get::<Option<String>, _>("index_started_at"),
            "speedwagons.index_started_at",
        )?;
        let indexed_at = Self::parse_optional_timestamp(
            row.get::<Option<String>, _>("indexed_at"),
            "speedwagons.indexed_at",
        )?;

        let provider_profile_id = Self::parse_optional_uuid(
            row.get::<Option<String>, _>("provider_profile_id"),
            "speedwagons.provider_profile_id",
        )?;

        Ok(Speedwagon {
            id,
            name: row.get::<String, _>("name"),
            description: row.get::<String, _>("description"),
            instruction: row.get::<Option<String>, _>("instruction"),
            lm: row.get::<Option<String>, _>("lm"),
            provider_profile_id,
            source_ids: vec![],
            index_dir: row.get::<Option<String>, _>("index_dir"),
            corpus_dir: row.get::<Option<String>, _>("corpus_dir"),
            index_status,
            index_error: row.get::<Option<String>, _>("index_error"),
            index_started_at,
            indexed_at,
            created_at,
            updated_at,
        })
    }

    async fn load_source_ids_for_speedwagon(
        &self,
        speedwagon_id: Uuid,
    ) -> RepositoryResult<Vec<Uuid>> {
        let rows = sqlx::query("SELECT source_id FROM speedwagon_sources WHERE speedwagon_id = ?;")
            .bind(speedwagon_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        rows.iter()
            .map(|row| {
                Self::parse_uuid(
                    row.get::<String, _>("source_id"),
                    "speedwagon_sources.source_id",
                )
            })
            .collect()
    }

    async fn load_speedwagon_by_id(&self, id: Uuid) -> RepositoryResult<Option<Speedwagon>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, description, instruction, lm, provider_profile_id, index_dir, corpus_dir,
                   index_status, index_error, index_started_at, indexed_at, created_at, updated_at
            FROM speedwagons
            WHERE id = ?;
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let mut sw = Self::row_to_speedwagon_without_sources(&row)?;
        sw.source_ids = self.load_source_ids_for_speedwagon(id).await?;
        Ok(Some(sw))
    }

    fn row_to_session_without_messages_or_relations(row: &SqliteRow) -> RepositoryResult<Session> {
        let id = Self::parse_uuid(row.get::<String, _>("id"), "sessions.id")?;
        let agent_id = Self::parse_uuid(row.get::<String, _>("agent_id"), "sessions.agent_id")?;
        let provider_profile_id = Self::parse_uuid(
            row.get::<String, _>("provider_profile_id"),
            "sessions.provider_profile_id",
        )?;
        let title = row.get::<Option<String>, _>("title");
        let created_at =
            Self::parse_timestamp(row.get::<String, _>("created_at"), "sessions.created_at")?;
        let updated_at =
            Self::parse_timestamp(row.get::<String, _>("updated_at"), "sessions.updated_at")?;

        Ok(Session {
            id,
            agent_id,
            provider_profile_id,
            title,
            messages: vec![],
            speedwagon_ids: vec![],
            source_ids: vec![],
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
            SELECT id, role, content, created_at
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
            let id = row.get::<String, _>("id");
            let role = Self::message_role_from_string(&row.get::<String, _>("role"))?;
            let content = row.get::<String, _>("content");
            let created_at = Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "session_messages.created_at",
            )?;
            messages.push(SessionMessage {
                id,
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
            SELECT id, agent_id, provider_profile_id, title, created_at, updated_at
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

        let mut session = Self::row_to_session_without_messages_or_relations(&row)?;
        session.messages = self.load_session_messages(session.id).await?;
        session.speedwagon_ids = self.get_session_speedwagon_ids(session.id).await?;
        session.source_ids = self.get_session_source_ids(session.id).await?;

        Ok(Some(session))
    }
}

#[async_trait]
impl Repository for SqliteRepository {
    async fn create_agent(&self, spec: AgentSpec) -> RepositoryResult<(Agent, bool)> {
        let now = Self::now_string();
        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let spec_json = normalize_spec(&spec)?;

        // Upsert: if another row with identical spec_json already exists, return it.
        // DO UPDATE (not DO NOTHING) is required for RETURNING to emit the conflict row.
        // The updated_at bump on conflict is a benign side effect; agents are immutable
        // under the copy-on-write model so updated_at has no semantic meaning.
        let row = sqlx::query(
            r#"
            INSERT INTO agents (id, spec_json, created_at, updated_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(spec_json) DO UPDATE SET updated_at = excluded.updated_at
            RETURNING id, spec_json, created_at, updated_at;
            "#,
        )
        .bind(&id_str)
        .bind(&spec_json)
        .bind(&now)
        .bind(&now)
        .fetch_one(&self.pool)
        .await?;

        let created = row.get::<String, _>("id") == id_str;
        let agent = Self::row_to_agent(&row)?;
        Ok((agent, created))
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
        title: Option<String>,
        speedwagon_ids: Vec<Uuid>,
        source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Session> {
        let now = Self::now_string();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO sessions (id, agent_id, provider_profile_id, title, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?);
            "#,
        )
        .bind(id.to_string())
        .bind(agent_id.to_string())
        .bind(provider_profile_id.to_string())
        .bind(title)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        self.set_session_speedwagons(id, speedwagon_ids).await?;
        self.set_session_sources(id, source_ids).await?;

        self.load_session_by_id(id)
            .await?
            .ok_or_else(|| RepositoryError::InvalidData("created session not found".to_string()))
    }

    async fn list_sessions(
        &self,
        agent_id: Option<Uuid>,
        include_messages: bool,
    ) -> RepositoryResult<Vec<Session>> {
        let rows = if let Some(agent_id) = agent_id {
            sqlx::query(
                r#"
                SELECT id, agent_id, provider_profile_id, title, created_at, updated_at
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
                SELECT id, agent_id, provider_profile_id, title, created_at, updated_at
                FROM sessions
                ORDER BY created_at ASC, id ASC;
                "#,
            )
            .fetch_all(&self.pool)
            .await?
        };

        let mut sessions = Vec::with_capacity(rows.len());
        for row in rows {
            let mut session = Self::row_to_session_without_messages_or_relations(&row)?;
            if include_messages {
                session.messages = self.load_session_messages(session.id).await?;
            }
            session.speedwagon_ids = self.get_session_speedwagon_ids(session.id).await?;
            session.source_ids = self.get_session_source_ids(session.id).await?;
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
    ) -> RepositoryResult<Option<SessionMessage>> {
        let mut tx = self.pool.begin().await?;

        let session_row = sqlx::query("SELECT title FROM sessions WHERE id = ?;")
            .bind(session_id.to_string())
            .fetch_optional(tx.as_mut())
            .await?;

        let Some(session_row) = session_row else {
            tx.rollback().await?;
            return Ok(None);
        };

        let now = Self::now_string();
        let msg_id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"
            INSERT INTO session_messages (id, session_id, role, content, created_at)
            VALUES (?, ?, ?, ?, ?);
            "#,
        )
        .bind(&msg_id)
        .bind(session_id.to_string())
        .bind(Self::message_role_to_string(&role))
        .bind(&content)
        .bind(&now)
        .execute(tx.as_mut())
        .await?;

        // Auto-set title from first user message if title is None
        let current_title = session_row.get::<Option<String>, _>("title");
        if matches!(role, MessageRole::User) && current_title.is_none() {
            let auto_title: String = content.chars().take(30).collect();
            sqlx::query("UPDATE sessions SET title = ?, updated_at = ? WHERE id = ?;")
                .bind(&auto_title)
                .bind(&now)
                .bind(session_id.to_string())
                .execute(tx.as_mut())
                .await?;
        } else {
            sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?;")
                .bind(&now)
                .bind(session_id.to_string())
                .execute(tx.as_mut())
                .await?;
        }

        tx.commit().await?;

        let created_at = Self::parse_timestamp(now, "created_at")?;
        Ok(Some(SessionMessage {
            id: msg_id,
            role,
            content,
            created_at,
        }))
    }

    async fn update_session_atomic(
        &self,
        id: Uuid,
        title: Option<String>,
        agent_id: Option<Uuid>,
        provider_profile_id: Option<Uuid>,
        speedwagon_ids: Option<Vec<Uuid>>,
        source_ids: Option<Vec<Uuid>>,
    ) -> RepositoryResult<Option<(Session, Option<Uuid>)>> {
        let id_str = id.to_string();

        let mut tx = self.pool.begin().await?;

        // Check session exists inside the transaction
        let exists: bool = sqlx::query_scalar("SELECT COUNT(*) > 0 FROM sessions WHERE id = ?")
            .bind(&id_str)
            .fetch_one(tx.as_mut())
            .await?;

        if !exists {
            tx.rollback().await?;
            return Ok(None);
        }

        let now = Self::now_string();

        if let Some(title) = title {
            sqlx::query("UPDATE sessions SET title = ?, updated_at = ? WHERE id = ?")
                .bind(&title)
                .bind(&now)
                .bind(&id_str)
                .execute(tx.as_mut())
                .await?;
        }

        // Track the previous agent_id only when the caller requested an agent change
        // and the new id actually differs from the current one. This lets the service
        // layer clean up an orphaned agent symmetrically with delete_session (CoW
        // pointer-swap cleanup). Skipping the SELECT on pure title/provider/speedwagon
        // updates avoids a wasted query on the common path.
        let mut previous_agent_id: Option<Uuid> = None;

        if let Some(new_agent_id) = agent_id {
            // Verify agent exists before updating
            let agent_exists: bool =
                sqlx::query_scalar("SELECT COUNT(*) > 0 FROM agents WHERE id = ?")
                    .bind(new_agent_id.to_string())
                    .fetch_one(tx.as_mut())
                    .await?;
            if !agent_exists {
                tx.rollback().await?;
                return Err(RepositoryError::InvalidData(format!(
                    "agent not found: {new_agent_id}"
                )));
            }

            // Read the current agent_id so we can report an orphan candidate to the
            // caller. Gated on `agent_id.is_some()` so the query only fires when an
            // agent swap was actually requested.
            let current_agent_str: String =
                sqlx::query_scalar("SELECT agent_id FROM sessions WHERE id = ?")
                    .bind(&id_str)
                    .fetch_one(tx.as_mut())
                    .await?;
            let current_agent_id = Uuid::parse_str(&current_agent_str).map_err(|e| {
                RepositoryError::InvalidData(format!("invalid agent_id in sessions row: {e}"))
            })?;

            if current_agent_id != new_agent_id {
                previous_agent_id = Some(current_agent_id);
                sqlx::query("UPDATE sessions SET agent_id = ?, updated_at = ? WHERE id = ?")
                    .bind(new_agent_id.to_string())
                    .bind(&now)
                    .bind(&id_str)
                    .execute(tx.as_mut())
                    .await?;
            }
        }

        if let Some(provider_profile_id) = provider_profile_id {
            sqlx::query("UPDATE sessions SET provider_profile_id = ?, updated_at = ? WHERE id = ?")
                .bind(provider_profile_id.to_string())
                .bind(&now)
                .bind(&id_str)
                .execute(tx.as_mut())
                .await?;
        }

        if let Some(speedwagon_ids) = speedwagon_ids {
            sqlx::query("DELETE FROM session_speedwagons WHERE session_id = ?")
                .bind(&id_str)
                .execute(tx.as_mut())
                .await?;
            for sw_id in &speedwagon_ids {
                sqlx::query(
                    "INSERT INTO session_speedwagons (session_id, speedwagon_id) VALUES (?, ?)",
                )
                .bind(&id_str)
                .bind(sw_id.to_string())
                .execute(tx.as_mut())
                .await?;
            }
            sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
                .bind(&now)
                .bind(&id_str)
                .execute(tx.as_mut())
                .await?;
        }

        if let Some(source_ids) = source_ids {
            sqlx::query("DELETE FROM session_sources WHERE session_id = ?")
                .bind(&id_str)
                .execute(tx.as_mut())
                .await?;
            for src_id in &source_ids {
                sqlx::query("INSERT INTO session_sources (session_id, source_id) VALUES (?, ?)")
                    .bind(&id_str)
                    .bind(src_id.to_string())
                    .execute(tx.as_mut())
                    .await?;
            }
            sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
                .bind(&now)
                .bind(&id_str)
                .execute(tx.as_mut())
                .await?;
        }

        tx.commit().await?;

        let session = self.get_session(id).await?;
        Ok(session.map(|s| (s, previous_agent_id)))
    }

    // --- Source ---

    async fn create_source(
        &self,
        name: String,
        source_type: SourceType,
        file_path: Option<String>,
        size: i64,
    ) -> RepositoryResult<Source> {
        let now = Self::now_string();
        let id = Uuid::new_v4();
        let source_type_str = match source_type {
            SourceType::LocalFile => "local_file",
        };

        sqlx::query(
            r#"
            INSERT INTO sources (id, name, source_type, file_path, size, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?);
            "#,
        )
        .bind(id.to_string())
        .bind(&name)
        .bind(source_type_str)
        .bind(&file_path)
        .bind(size)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(Source {
            id,
            name,
            source_type,
            file_path,
            size,
            created_at: Self::parse_timestamp(now.clone(), "sources.created_at")?,
            updated_at: Self::parse_timestamp(now, "sources.updated_at")?,
        })
    }

    async fn list_sources(&self) -> RepositoryResult<Vec<Source>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, source_type, file_path, size, created_at, updated_at
            FROM sources
            ORDER BY created_at DESC;
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(Self::row_to_source).collect()
    }

    async fn get_source(&self, id: Uuid) -> RepositoryResult<Option<Source>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, source_type, file_path, size, created_at, updated_at
            FROM sources
            WHERE id = ?;
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.as_ref().map(Self::row_to_source).transpose()
    }

    async fn delete_source(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM sources WHERE id = ?;")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // --- Speedwagon ---

    async fn create_speedwagon(
        &self,
        name: String,
        description: String,
        instruction: Option<String>,
        lm: Option<String>,
        provider_profile_id: Option<Uuid>,
        source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Speedwagon> {
        let now = Self::now_string();
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO speedwagons
                (id, name, description, instruction, lm, provider_profile_id, index_status, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, 'not_indexed', ?, ?);
            "#,
        )
        .bind(id.to_string())
        .bind(&name)
        .bind(&description)
        .bind(&instruction)
        .bind(&lm)
        .bind(provider_profile_id.map(|u| u.to_string()))
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        for source_id in &source_ids {
            sqlx::query("INSERT INTO speedwagon_sources (speedwagon_id, source_id) VALUES (?, ?);")
                .bind(id.to_string())
                .bind(source_id.to_string())
                .execute(&self.pool)
                .await?;
        }

        self.load_speedwagon_by_id(id)
            .await?
            .ok_or_else(|| RepositoryError::InvalidData("created speedwagon not found".to_string()))
    }

    async fn list_speedwagons(&self) -> RepositoryResult<Vec<Speedwagon>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, description, instruction, lm, provider_profile_id, index_dir, corpus_dir,
                   index_status, index_error, index_started_at, indexed_at, created_at, updated_at
            FROM speedwagons
            ORDER BY created_at DESC;
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        // Bulk-load all speedwagon_sources into HashMap<speedwagon_id, [source_id]>
        let source_rows = sqlx::query("SELECT speedwagon_id, source_id FROM speedwagon_sources;")
            .fetch_all(&self.pool)
            .await?;

        let mut source_map: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for sr in &source_rows {
            let sw_id = Self::parse_uuid(
                sr.get::<String, _>("speedwagon_id"),
                "speedwagon_sources.speedwagon_id",
            )?;
            let src_id = Self::parse_uuid(
                sr.get::<String, _>("source_id"),
                "speedwagon_sources.source_id",
            )?;
            source_map.entry(sw_id).or_default().push(src_id);
        }

        let mut speedwagons = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut sw = Self::row_to_speedwagon_without_sources(row)?;
            // remove: takes ownership from HashMap, avoiding clone
            sw.source_ids = source_map.remove(&sw.id).unwrap_or_default();
            speedwagons.push(sw);
        }

        Ok(speedwagons)
    }

    async fn get_speedwagon(&self, id: Uuid) -> RepositoryResult<Option<Speedwagon>> {
        self.load_speedwagon_by_id(id).await
    }

    async fn update_speedwagon(
        &self,
        id: Uuid,
        name: String,
        description: String,
        instruction: Option<String>,
        lm: Option<String>,
        provider_profile_id: Option<Uuid>,
        source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Option<Speedwagon>> {
        let now = Self::now_string();

        // Check if source_ids changed to decide whether to reset index_status
        let existing_source_ids = self.load_source_ids_for_speedwagon(id).await?;
        let sources_changed = {
            let mut old = existing_source_ids.clone();
            let mut new = source_ids.clone();
            old.sort();
            new.sort();
            old != new
        };

        let result = if sources_changed {
            sqlx::query(
                r#"
                UPDATE speedwagons
                SET name = ?, description = ?, instruction = ?, lm = ?,
                    provider_profile_id = ?,
                    index_status = 'not_indexed', index_error = NULL,
                    updated_at = ?
                WHERE id = ?;
                "#,
            )
            .bind(&name)
            .bind(&description)
            .bind(&instruction)
            .bind(&lm)
            .bind(provider_profile_id.map(|u| u.to_string()))
            .bind(&now)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                UPDATE speedwagons
                SET name = ?, description = ?, instruction = ?, lm = ?,
                    provider_profile_id = ?,
                    updated_at = ?
                WHERE id = ?;
                "#,
            )
            .bind(&name)
            .bind(&description)
            .bind(&instruction)
            .bind(&lm)
            .bind(provider_profile_id.map(|u| u.to_string()))
            .bind(&now)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?
        };

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        // Replace speedwagon_sources entirely
        sqlx::query("DELETE FROM speedwagon_sources WHERE speedwagon_id = ?;")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        for source_id in &source_ids {
            sqlx::query("INSERT INTO speedwagon_sources (speedwagon_id, source_id) VALUES (?, ?);")
                .bind(id.to_string())
                .bind(source_id.to_string())
                .execute(&self.pool)
                .await?;
        }

        self.load_speedwagon_by_id(id).await
    }

    async fn delete_speedwagon(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM speedwagons WHERE id = ?;")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn update_speedwagon_index_status(
        &self,
        id: Uuid,
        status: SpeedwagonIndexStatus,
        error: Option<String>,
        index_dir: Option<String>,
        corpus_dir: Option<String>,
        index_started_at: Option<DateTime<Utc>>,
        indexed_at: Option<DateTime<Utc>>,
    ) -> RepositoryResult<bool> {
        let now = Self::now_string();
        let status_str = Self::index_status_to_string(&status);
        let index_started_at_str =
            index_started_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true));
        let indexed_at_str = indexed_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true));

        let result = sqlx::query(
            r#"
            UPDATE speedwagons
            SET index_status = ?, index_error = ?, index_dir = ?, corpus_dir = ?,
                index_started_at = ?, indexed_at = ?, updated_at = ?
            WHERE id = ?;
            "#,
        )
        .bind(status_str)
        .bind(error)
        .bind(index_dir)
        .bind(corpus_dir)
        .bind(index_started_at_str)
        .bind(indexed_at_str)
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // --- Session <-> Speedwagon/Source relationships ---

    async fn set_session_speedwagons(
        &self,
        session_id: Uuid,
        speedwagon_ids: Vec<Uuid>,
    ) -> RepositoryResult<()> {
        sqlx::query("DELETE FROM session_speedwagons WHERE session_id = ?;")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        for speedwagon_id in &speedwagon_ids {
            sqlx::query(
                "INSERT INTO session_speedwagons (session_id, speedwagon_id) VALUES (?, ?);",
            )
            .bind(session_id.to_string())
            .bind(speedwagon_id.to_string())
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    async fn get_session_speedwagon_ids(&self, session_id: Uuid) -> RepositoryResult<Vec<Uuid>> {
        let rows =
            sqlx::query("SELECT speedwagon_id FROM session_speedwagons WHERE session_id = ?;")
                .bind(session_id.to_string())
                .fetch_all(&self.pool)
                .await?;

        rows.iter()
            .map(|row| {
                Self::parse_uuid(
                    row.get::<String, _>("speedwagon_id"),
                    "session_speedwagons.speedwagon_id",
                )
            })
            .collect()
    }

    async fn set_session_sources(
        &self,
        session_id: Uuid,
        source_ids: Vec<Uuid>,
    ) -> RepositoryResult<()> {
        sqlx::query("DELETE FROM session_sources WHERE session_id = ?;")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        for source_id in &source_ids {
            sqlx::query("INSERT INTO session_sources (session_id, source_id) VALUES (?, ?);")
                .bind(session_id.to_string())
                .bind(source_id.to_string())
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    async fn get_session_source_ids(&self, session_id: Uuid) -> RepositoryResult<Vec<Uuid>> {
        let rows = sqlx::query("SELECT source_id FROM session_sources WHERE session_id = ?;")
            .bind(session_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        rows.iter()
            .map(|row| {
                Self::parse_uuid(
                    row.get::<String, _>("source_id"),
                    "session_sources.source_id",
                )
            })
            .collect()
    }

    async fn get_sessions_by_speedwagon_id(
        &self,
        speedwagon_id: Uuid,
    ) -> RepositoryResult<Vec<Uuid>> {
        let rows =
            sqlx::query("SELECT session_id FROM session_speedwagons WHERE speedwagon_id = ?;")
                .bind(speedwagon_id.to_string())
                .fetch_all(&self.pool)
                .await?;

        rows.iter()
            .map(|row| {
                Self::parse_uuid(
                    row.get::<String, _>("session_id"),
                    "session_speedwagons.session_id",
                )
            })
            .collect()
    }

    // --- Session Tool Calls ---

    async fn save_tool_calls(
        &self,
        message_id: &str,
        tool_calls: &[SessionToolCall],
    ) -> RepositoryResult<()> {
        for tc in tool_calls {
            let tool_args_json = tc
                .tool_args
                .as_ref()
                .map(|v| serde_json::to_string(v))
                .transpose()?;
            let tool_result_json = tc
                .tool_result
                .as_ref()
                .map(|v| serde_json::to_string(v))
                .transpose()?;
            sqlx::query(
                r#"
                INSERT INTO session_tool_calls
                    (id, message_id, tool_name, tool_args, tool_result, duration_ms, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?);
                "#,
            )
            .bind(&tc.id)
            .bind(message_id)
            .bind(&tc.tool_name)
            .bind(tool_args_json)
            .bind(tool_result_json)
            .bind(tc.duration_ms)
            .bind(tc.created_at.to_rfc3339_opts(SecondsFormat::Millis, true))
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn get_tool_calls_for_message(
        &self,
        message_id: &str,
    ) -> RepositoryResult<Vec<SessionToolCall>> {
        let rows = sqlx::query(
            r#"
            SELECT id, message_id, tool_name, tool_args, tool_result, duration_ms, created_at
            FROM session_tool_calls
            WHERE message_id = ?
            ORDER BY created_at ASC, id ASC;
            "#,
        )
        .bind(message_id)
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let tool_args = row
                .get::<Option<String>, _>("tool_args")
                .map(|s| serde_json::from_str::<serde_json::Value>(&s))
                .transpose()?;
            let tool_result = row
                .get::<Option<String>, _>("tool_result")
                .map(|s| serde_json::from_str::<serde_json::Value>(&s))
                .transpose()?;
            let created_at = Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "session_tool_calls.created_at",
            )?;
            result.push(SessionToolCall {
                id: row.get::<String, _>("id"),
                message_id: row.get::<String, _>("message_id"),
                tool_name: row.get::<String, _>("tool_name"),
                tool_args,
                tool_result,
                duration_ms: row.get::<Option<i64>, _>("duration_ms"),
                created_at,
            });
        }
        Ok(result)
    }

    async fn get_tool_calls_for_session(
        &self,
        session_id: Uuid,
    ) -> RepositoryResult<Vec<SessionToolCall>> {
        let rows = sqlx::query(
            r#"
            SELECT tc.id, tc.message_id, tc.tool_name, tc.tool_args, tc.tool_result,
                   tc.duration_ms, tc.created_at
            FROM session_tool_calls tc
            WHERE tc.message_id IN (
                SELECT id FROM session_messages WHERE session_id = ?
            )
            ORDER BY tc.created_at ASC, tc.id ASC;
            "#,
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let tool_args = row
                .get::<Option<String>, _>("tool_args")
                .map(|s| serde_json::from_str::<serde_json::Value>(&s))
                .transpose()?;
            let tool_result = row
                .get::<Option<String>, _>("tool_result")
                .map(|s| serde_json::from_str::<serde_json::Value>(&s))
                .transpose()?;
            let created_at = Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "session_tool_calls.created_at",
            )?;
            result.push(SessionToolCall {
                id: row.get::<String, _>("id"),
                message_id: row.get::<String, _>("message_id"),
                tool_name: row.get::<String, _>("tool_name"),
                tool_args,
                tool_result,
                duration_ms: row.get::<Option<i64>, _>("duration_ms"),
                created_at,
            });
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tempfile::tempdir;

    use super::SqliteRepository;
    use crate::models::MessageRole;
    use crate::repository::Repository;
    use ailoy::{AgentProvider, AgentSpec, LangModelProvider};

    #[tokio::test]
    async fn sqlite_persists_data_between_repository_instances() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("app.db");
        let db_url = format!("sqlite://{}", db_path.display());

        let repository = SqliteRepository::new(&db_url)
            .await
            .expect("sqlite repository should be created");

        let (agent, _) = repository
            .create_agent(AgentSpec::new("gpt-4.1"))
            .await
            .expect("agent should be created");

        let provider = AgentProvider {
            lm: LangModelProvider::openai("test-key".into()),
            tools: vec![],
        };

        let profile = repository
            .create_provider_profile("test-profile".to_string(), provider, true)
            .await
            .expect("provider profile should be created");

        let session = repository
            .create_session(agent.id, profile.id, None, vec![], vec![])
            .await
            .expect("session should be created");

        let session2 = SqliteRepository::new(&db_url)
            .await
            .expect("second instance should be created")
            .get_session(session.id)
            .await
            .expect("session should be fetched")
            .expect("session should exist");

        assert_eq!(session.id, session2.id);
        assert_eq!(session.agent_id, session2.agent_id);

        repository
            .add_session_message(session.id, MessageRole::User, "hello world".to_string())
            .await
            .expect("message should be added");

        let updated_session = SqliteRepository::new(&db_url)
            .await
            .expect("third instance should be created")
            .get_session(session.id)
            .await
            .expect("session should be fetched")
            .expect("session should exist");

        assert_eq!(updated_session.messages.len(), 1);
        assert_eq!(updated_session.title, Some("hello world".to_string()));
    }

    /// Concurrent `create_agent` calls with identical specs must converge
    /// to a single row via the `UNIQUE(spec_json)` + `ON CONFLICT` upsert.
    /// Verifies Finding #3 (CoW Principle E: DB-level atomicity).
    #[tokio::test]
    async fn concurrent_create_agent_deduplicates_by_spec() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("app.db");
        let db_url = format!("sqlite://{}", db_path.display());

        let repository = Arc::new(
            SqliteRepository::new(&db_url)
                .await
                .expect("sqlite repository should be created"),
        );

        let concurrency = 10;
        let handles: Vec<_> = (0..concurrency)
            .map(|_| {
                let repo = Arc::clone(&repository);
                tokio::spawn(async move { repo.create_agent(AgentSpec::new("gpt-4.1")).await })
            })
            .collect();

        let mut ids = std::collections::HashSet::new();
        for handle in handles {
            let (agent, _) = handle
                .await
                .expect("task should join")
                .expect("agent should be returned");
            ids.insert(agent.id);
        }

        assert_eq!(ids.len(), 1, "all concurrent calls should return the same agent id");

        let agents = repository
            .list_agents()
            .await
            .expect("agents should be listed");
        assert_eq!(agents.len(), 1, "exactly one agent row should exist in DB");
    }
}
