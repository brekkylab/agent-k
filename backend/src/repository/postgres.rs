use async_trait::async_trait;
use uuid::Uuid;

use crate::models::{Agent, MessageRole, ProviderProfile, Session};
use crate::repository::{Repository, RepositoryResult};
use ailoy::{AgentProvider, AgentSpec};

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
    ) -> RepositoryResult<Session> {
        todo!("postgres implementation")
    }

    async fn list_sessions(&self, _agent_id: Option<Uuid>) -> RepositoryResult<Vec<Session>> {
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
    ) -> RepositoryResult<Option<Session>> {
        todo!("postgres implementation")
    }
}
