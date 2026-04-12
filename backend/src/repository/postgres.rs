use ailoy::{AgentProvider, AgentSpec};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::models::{
    Agent, MessageRole, ProviderProfile, Session, SessionMessage, SessionToolCall, Source,
    SourceType, Speedwagon, SpeedwagonIndexStatus,
};
use crate::repository::{Repository, RepositoryResult};

pub struct PostgresRepository;

impl PostgresRepository {
    pub async fn new(_database_url: &str) -> RepositoryResult<Self> {
        todo!("postgres implementation")
    }
}

#[async_trait]
impl Repository for PostgresRepository {
    async fn create_agent(&self, _spec: AgentSpec) -> RepositoryResult<Agent> {
        todo!("postgres implementation")
    }

    async fn list_agents(&self) -> RepositoryResult<Vec<Agent>> {
        todo!("postgres implementation")
    }

    async fn get_agent(&self, _id: Uuid) -> RepositoryResult<Option<Agent>> {
        todo!("postgres implementation")
    }

    async fn find_agent_by_spec(
        &self,
        _normalized_spec_json: &str,
    ) -> RepositoryResult<Option<Agent>> {
        todo!("postgres implementation")
    }

    async fn update_agent(&self, _id: Uuid, _spec: AgentSpec) -> RepositoryResult<Option<Agent>> {
        todo!("postgres implementation")
    }

    async fn delete_agent(&self, _id: Uuid) -> RepositoryResult<bool> {
        todo!("postgres implementation")
    }

    async fn has_sessions_for_agent(&self, _agent_id: Uuid) -> RepositoryResult<bool> {
        todo!("postgres implementation")
    }

    async fn create_provider_profile(
        &self,
        _name: String,
        _provider: AgentProvider,
        _is_default: bool,
    ) -> RepositoryResult<ProviderProfile> {
        todo!("postgres implementation")
    }

    async fn list_provider_profiles(&self) -> RepositoryResult<Vec<ProviderProfile>> {
        todo!("postgres implementation")
    }

    async fn get_provider_profile(&self, _id: Uuid) -> RepositoryResult<Option<ProviderProfile>> {
        todo!("postgres implementation")
    }

    async fn update_provider_profile(
        &self,
        _id: Uuid,
        _name: String,
        _provider: AgentProvider,
        _is_default: bool,
    ) -> RepositoryResult<Option<ProviderProfile>> {
        todo!("postgres implementation")
    }

    async fn delete_provider_profile(&self, _id: Uuid) -> RepositoryResult<bool> {
        todo!("postgres implementation")
    }

    async fn has_sessions_for_provider_profile(
        &self,
        _provider_profile_id: Uuid,
    ) -> RepositoryResult<bool> {
        todo!("postgres implementation")
    }

    async fn upsert_provider_profile_by_name(
        &self,
        _name: &str,
        _provider: AgentProvider,
        _is_default: bool,
    ) -> RepositoryResult<ProviderProfile> {
        todo!("postgres implementation")
    }

    async fn create_session(
        &self,
        _agent_id: Uuid,
        _provider_profile_id: Uuid,
        _title: Option<String>,
        _speedwagon_ids: Vec<Uuid>,
        _source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Session> {
        todo!("postgres implementation")
    }

    async fn list_sessions(
        &self,
        _agent_id: Option<Uuid>,
        _include_messages: bool,
    ) -> RepositoryResult<Vec<Session>> {
        todo!("postgres implementation")
    }

    async fn get_session(&self, _id: Uuid) -> RepositoryResult<Option<Session>> {
        todo!("postgres implementation")
    }

    async fn delete_session(&self, _id: Uuid) -> RepositoryResult<bool> {
        todo!("postgres implementation")
    }

    async fn add_session_message(
        &self,
        _session_id: Uuid,
        _role: MessageRole,
        _content: String,
    ) -> RepositoryResult<Option<SessionMessage>> {
        todo!("postgres implementation")
    }

    async fn update_session_atomic(
        &self,
        _id: Uuid,
        _title: Option<String>,
        _agent_id: Option<Uuid>,
        _provider_profile_id: Option<Uuid>,
        _speedwagon_ids: Option<Vec<Uuid>>,
        _source_ids: Option<Vec<Uuid>>,
    ) -> RepositoryResult<Option<Session>> {
        todo!("postgres implementation")
    }

    // --- Source ---

    async fn create_source(
        &self,
        _name: String,
        _source_type: SourceType,
        _file_path: Option<String>,
        _size: i64,
    ) -> RepositoryResult<Source> {
        todo!("postgres implementation")
    }

    async fn list_sources(&self) -> RepositoryResult<Vec<Source>> {
        todo!("postgres implementation")
    }

    async fn get_source(&self, _id: Uuid) -> RepositoryResult<Option<Source>> {
        todo!("postgres implementation")
    }

    async fn delete_source(&self, _id: Uuid) -> RepositoryResult<bool> {
        todo!("postgres implementation")
    }

    // --- Speedwagon ---

    async fn create_speedwagon(
        &self,
        _name: String,
        _description: String,
        _instruction: Option<String>,
        _lm: Option<String>,
        _provider_profile_id: Option<Uuid>,
        _source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Speedwagon> {
        todo!("postgres implementation")
    }

    async fn list_speedwagons(&self) -> RepositoryResult<Vec<Speedwagon>> {
        todo!("postgres implementation")
    }

    async fn get_speedwagon(&self, _id: Uuid) -> RepositoryResult<Option<Speedwagon>> {
        todo!("postgres implementation")
    }

    async fn update_speedwagon(
        &self,
        _id: Uuid,
        _name: String,
        _description: String,
        _instruction: Option<String>,
        _lm: Option<String>,
        _provider_profile_id: Option<Uuid>,
        _source_ids: Vec<Uuid>,
    ) -> RepositoryResult<Option<Speedwagon>> {
        todo!("postgres implementation")
    }

    async fn delete_speedwagon(&self, _id: Uuid) -> RepositoryResult<bool> {
        todo!("postgres implementation")
    }

    async fn update_speedwagon_index_status(
        &self,
        _id: Uuid,
        _status: SpeedwagonIndexStatus,
        _error: Option<String>,
        _index_dir: Option<String>,
        _corpus_dir: Option<String>,
        _index_started_at: Option<DateTime<Utc>>,
        _indexed_at: Option<DateTime<Utc>>,
    ) -> RepositoryResult<bool> {
        todo!("postgres implementation")
    }

    // --- Session <-> Speedwagon/Source relationships ---

    async fn set_session_speedwagons(
        &self,
        _session_id: Uuid,
        _speedwagon_ids: Vec<Uuid>,
    ) -> RepositoryResult<()> {
        todo!("postgres implementation")
    }

    async fn get_session_speedwagon_ids(&self, _session_id: Uuid) -> RepositoryResult<Vec<Uuid>> {
        todo!("postgres implementation")
    }

    async fn set_session_sources(
        &self,
        _session_id: Uuid,
        _source_ids: Vec<Uuid>,
    ) -> RepositoryResult<()> {
        todo!("postgres implementation")
    }

    async fn get_session_source_ids(&self, _session_id: Uuid) -> RepositoryResult<Vec<Uuid>> {
        todo!("postgres implementation")
    }

    async fn get_sessions_by_speedwagon_id(
        &self,
        _speedwagon_id: Uuid,
    ) -> RepositoryResult<Vec<Uuid>> {
        todo!("postgres implementation")
    }

    // --- Session Tool Calls ---

    async fn save_tool_calls(
        &self,
        _message_id: &str,
        _tool_calls: &[SessionToolCall],
    ) -> RepositoryResult<()> {
        todo!("postgres implementation")
    }

    async fn get_tool_calls_for_message(
        &self,
        _message_id: &str,
    ) -> RepositoryResult<Vec<SessionToolCall>> {
        todo!("postgres implementation")
    }

    async fn get_tool_calls_for_session(
        &self,
        _session_id: Uuid,
    ) -> RepositoryResult<Vec<SessionToolCall>> {
        todo!("postgres implementation")
    }
}
