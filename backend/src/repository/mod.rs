mod postgres;
mod sqlite;

use std::sync::Arc;

use ailoy::{AgentProvider, AgentSpec};
use async_trait::async_trait;
use chrono::DateTime;
use chrono::Utc;
use thiserror::Error;
use uuid::Uuid;

use crate::models::{
    Agent, MessageRole, ProviderProfile, Session, SessionMessage, SessionToolCall, Source,
    SourceType, Speedwagon, SpeedwagonIndexStatus,
};

pub use postgres::PostgresRepository;
pub use sqlite::SqliteRepository;

const DEFAULT_DATABASE_URL: &str = "sqlite://./data/app.db";

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

#[async_trait]
pub trait Repository: Send + Sync {
    async fn create_agent(&self, spec: AgentSpec) -> RepositoryResult<Agent>;
    async fn list_agents(&self) -> RepositoryResult<Vec<Agent>>;
    async fn get_agent(&self, id: Uuid) -> RepositoryResult<Option<Agent>>;
    async fn update_agent(&self, id: Uuid, spec: AgentSpec) -> RepositoryResult<Option<Agent>>;
    async fn delete_agent(&self, id: Uuid) -> RepositoryResult<bool>;
    async fn has_sessions_for_agent(&self, agent_id: Uuid) -> RepositoryResult<bool>;

    async fn create_provider_profile(
        &self,
        name: String,
        provider: AgentProvider,
        is_default: bool,
    ) -> RepositoryResult<ProviderProfile>;
    async fn list_provider_profiles(&self) -> RepositoryResult<Vec<ProviderProfile>>;
    async fn get_provider_profile(&self, id: Uuid) -> RepositoryResult<Option<ProviderProfile>>;
    async fn update_provider_profile(
        &self,
        id: Uuid,
        name: String,
        provider: AgentProvider,
        is_default: bool,
    ) -> RepositoryResult<Option<ProviderProfile>>;
    async fn delete_provider_profile(&self, id: Uuid) -> RepositoryResult<bool>;
    async fn has_sessions_for_provider_profile(
        &self,
        provider_profile_id: Uuid,
    ) -> RepositoryResult<bool>;

    async fn upsert_provider_profile_by_name(
        &self,
        name: &str,
        provider: AgentProvider,
        is_default: bool,
    ) -> RepositoryResult<ProviderProfile>;

    async fn create_session(
        &self,
        agent_id: Uuid,
        provider_profile_id: Uuid,
        title: Option<String>,
        speedwagon_ids: Vec<Uuid>,
        source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Session>;
    async fn list_sessions(
        &self,
        agent_id: Option<Uuid>,
        include_messages: bool,
    ) -> RepositoryResult<Vec<Session>>;
    async fn get_session(&self, id: Uuid) -> RepositoryResult<Option<Session>>;
    async fn delete_session(&self, id: Uuid) -> RepositoryResult<bool>;
    async fn add_session_message(
        &self,
        session_id: Uuid,
        role: MessageRole,
        content: String,
    ) -> RepositoryResult<Option<SessionMessage>>;
    async fn update_session_atomic(
        &self,
        id: Uuid,
        title: Option<String>,
        provider_profile_id: Option<Uuid>,
        speedwagon_ids: Option<Vec<Uuid>>,
        source_ids: Option<Vec<Uuid>>,
    ) -> RepositoryResult<Option<Session>>;

    // --- Source ---
    async fn create_source(
        &self,
        name: String,
        source_type: SourceType,
        file_path: Option<String>,
        size: i64,
    ) -> RepositoryResult<Source>;
    async fn list_sources(&self) -> RepositoryResult<Vec<Source>>;
    async fn get_source(&self, id: Uuid) -> RepositoryResult<Option<Source>>;
    async fn delete_source(&self, id: Uuid) -> RepositoryResult<bool>;

    // --- Speedwagon ---
    async fn create_speedwagon(
        &self,
        name: String,
        description: String,
        instruction: Option<String>,
        lm: Option<String>,
        provider_profile_id: Option<Uuid>,
        source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Speedwagon>;
    async fn list_speedwagons(&self) -> RepositoryResult<Vec<Speedwagon>>;
    async fn get_speedwagon(&self, id: Uuid) -> RepositoryResult<Option<Speedwagon>>;
    async fn update_speedwagon(
        &self,
        id: Uuid,
        name: String,
        description: String,
        instruction: Option<String>,
        lm: Option<String>,
        provider_profile_id: Option<Uuid>,
        source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Option<Speedwagon>>;
    async fn delete_speedwagon(&self, id: Uuid) -> RepositoryResult<bool>;
    async fn update_speedwagon_index_status(
        &self,
        id: Uuid,
        status: SpeedwagonIndexStatus,
        error: Option<String>,
        index_dir: Option<String>,
        corpus_dir: Option<String>,
        index_started_at: Option<DateTime<Utc>>,
        indexed_at: Option<DateTime<Utc>>,
    ) -> RepositoryResult<bool>;

    // --- Session <-> Speedwagon/Source relationships ---
    async fn set_session_speedwagons(
        &self,
        session_id: Uuid,
        speedwagon_ids: Vec<Uuid>,
    ) -> RepositoryResult<()>;
    async fn get_session_speedwagon_ids(&self, session_id: Uuid) -> RepositoryResult<Vec<Uuid>>;
    async fn set_session_sources(
        &self,
        session_id: Uuid,
        source_ids: Vec<Uuid>,
    ) -> RepositoryResult<()>;
    async fn get_session_source_ids(&self, session_id: Uuid) -> RepositoryResult<Vec<Uuid>>;
    async fn get_sessions_by_speedwagon_id(
        &self,
        speedwagon_id: Uuid,
    ) -> RepositoryResult<Vec<Uuid>>;

    // --- Session Tool Calls ---
    async fn save_tool_calls(
        &self,
        message_id: &str,
        tool_calls: &[SessionToolCall],
    ) -> RepositoryResult<()>;

    /// Currently unused — will be needed when message-level pagination is introduced,
    /// allowing lazy-load of tool call details on user expand.
    #[allow(dead_code)]
    async fn get_tool_calls_for_message(
        &self,
        message_id: &str,
    ) -> RepositoryResult<Vec<SessionToolCall>>;

    async fn get_tool_calls_for_session(
        &self,
        session_id: Uuid,
    ) -> RepositoryResult<Vec<SessionToolCall>>;
}

pub async fn create_repository_from_env() -> RepositoryResult<Arc<dyn Repository>> {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    if database_url == DEFAULT_DATABASE_URL {
        std::fs::create_dir_all("./data").map_err(|error| {
            RepositoryError::InvalidData(format!("failed to create data directory: {error}"))
        })?;
    }
    create_repository(&database_url).await
}

pub async fn create_repository(database_url: &str) -> RepositoryResult<Arc<dyn Repository>> {
    if database_url.starts_with("sqlite:") {
        let repository = SqliteRepository::new(database_url).await?;
        return Ok(Arc::new(repository));
    }

    if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
        let repository = PostgresRepository::new(database_url).await?;
        return Ok(Arc::new(repository));
    }

    Err(RepositoryError::InvalidDatabaseUrl(
        database_url.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::create_repository;

    #[tokio::test]
    #[should_panic(expected = "postgres implementation")]
    async fn postgres_branch_is_explicit_todo() {
        let _ = create_repository("postgres://localhost/agentwebui_test").await;
    }
}
