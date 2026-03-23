use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ailoy::{AgentProvider, AgentRuntime, LangModelAPISchema, LangModelProvider, ToolSet};
use tokio::sync::Mutex as TokioMutex;
use url::Url;
use uuid::Uuid;

use crate::models::Session;
use crate::repository::{
    Repository, RepositoryError, RepositoryResult, create_repository, create_repository_from_env,
};

#[derive(Clone)]
struct CachedRuntime {
    agent_id: Uuid,
    provider_profile_id: Uuid,
    runtime: Arc<TokioMutex<AgentRuntime>>,
}

pub struct AppState {
    pub repository: Arc<dyn Repository>,
    runtime_cache: Mutex<HashMap<Uuid, CachedRuntime>>,
}

impl AppState {
    pub async fn new() -> std::io::Result<Self> {
        let repository = create_repository_from_env().await.map_err(to_io_error)?;
        let state = Self {
            repository,
            runtime_cache: Mutex::new(HashMap::new()),
        };
        state
            .overwrite_default_provider_profiles_from_env()
            .await
            .map_err(to_io_error)?;
        Ok(state)
    }

    #[cfg(test)]
    pub async fn new_without_bootstrap(database_url: &str) -> RepositoryResult<Self> {
        let repository = create_repository(database_url).await?;
        Ok(Self {
            repository,
            runtime_cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn invalidate_session_runtime(&self, session_id: Uuid) {
        if let Ok(mut cache) = self.runtime_cache.lock() {
            cache.remove(&session_id);
        }
    }

    pub fn invalidate_runtimes_by_agent_id(&self, agent_id: Uuid) {
        if let Ok(mut cache) = self.runtime_cache.lock() {
            cache.retain(|_, runtime| runtime.agent_id != agent_id);
        }
    }

    pub fn invalidate_runtimes_by_provider_profile_id(&self, provider_profile_id: Uuid) {
        if let Ok(mut cache) = self.runtime_cache.lock() {
            cache.retain(|_, runtime| runtime.provider_profile_id != provider_profile_id);
        }
    }

    pub async fn get_or_create_runtime_for_session(
        &self,
        session: &Session,
    ) -> RepositoryResult<Arc<TokioMutex<AgentRuntime>>> {
        if let Ok(mut cache) = self.runtime_cache.lock()
            && let Some(cached) = cache.get(&session.id).cloned()
        {
            if cached.agent_id == session.agent_id
                && cached.provider_profile_id == session.provider_profile_id
            {
                return Ok(cached.runtime);
            }
            cache.remove(&session.id);
        }

        let agent = self
            .repository
            .get_agent(session.agent_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::InvalidData("agent not found for session".to_string())
            })?;

        let provider_profile = self
            .repository
            .get_provider_profile(session.provider_profile_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::InvalidData("provider profile not found for session".to_string())
            })?;

        let runtime = Arc::new(TokioMutex::new(AgentRuntime::new(
            agent.spec,
            provider_profile.provider,
            ToolSet::new(),
        )));

        let cached = CachedRuntime {
            agent_id: session.agent_id,
            provider_profile_id: session.provider_profile_id,
            runtime: Arc::clone(&runtime),
        };

        if let Ok(mut cache) = self.runtime_cache.lock() {
            cache.insert(session.id, cached);
        }

        Ok(runtime)
    }

    async fn overwrite_default_provider_profiles_from_env(&self) -> RepositoryResult<()> {
        let profile_definitions = [
            (
                "OPENAI_API_KEY",
                "openai-default",
                LangModelAPISchema::ChatCompletion,
                "https://api.openai.com/v1/chat/completions",
            ),
            (
                "ANTHROPIC_API_KEY",
                "anthropic-default",
                LangModelAPISchema::Anthropic,
                "https://api.anthropic.com/v1/messages",
            ),
            (
                "GEMINI_API_KEY",
                "gemini-default",
                LangModelAPISchema::Gemini,
                "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent",
            ),
        ];

        for (env_key, name, schema, url) in profile_definitions {
            let Ok(api_key) = std::env::var(env_key) else {
                continue;
            };
            if api_key.trim().is_empty() {
                continue;
            }

            let parsed_url = Url::parse(url)
                .map_err(|_| RepositoryError::InvalidData(format!("invalid URL for `{name}`")))?;

            let provider = AgentProvider {
                lm: LangModelProvider::API {
                    schema,
                    url: parsed_url,
                    api_key: Some(api_key),
                },
                tools: vec![],
            };

            self.repository
                .upsert_provider_profile_by_name(name, provider, true)
                .await?;
        }

        Ok(())
    }
}

fn to_io_error(error: RepositoryError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
