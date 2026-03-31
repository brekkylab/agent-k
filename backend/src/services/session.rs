use ailoy::{AgentProvider, LangModelAPISchema, LangModelProvider};
use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError};
use chat_agent::ChatAgentRunError;
use uuid::Uuid;

use crate::models::{
    AddSessionMessageResponse, CreateSessionRequest, ErrorResponse, MessageRole, Session,
    UpdateSessionRequest,
};
use crate::repository::RepositoryError;
use crate::state::AppState;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session not found")]
    NotFound,
    #[error("agent not found")]
    AgentNotFound,
    #[error("provider profile not found")]
    ProviderProfileNotFound,
    #[error("no default provider profile available")]
    NoDefaultProviderProfile,
    #[error("message content is empty")]
    EmptyContent,
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("runtime error: {0}")]
    Runtime(String),
}

impl ResponseError for SessionError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound | Self::AgentNotFound | Self::ProviderProfileNotFound => {
                StatusCode::NOT_FOUND
            }
            Self::NoDefaultProviderProfile | Self::EmptyContent => StatusCode::BAD_REQUEST,
            Self::Runtime(_) => StatusCode::BAD_GATEWAY,
            Self::Repository(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let Self::Repository(e) = self {
            eprintln!("repository error: {e}");
            return HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR)
                .json(ErrorResponse { error: "internal server error".to_string() });
        }
        HttpResponse::build(self.status_code()).json(ErrorResponse {
            error: self.to_string(),
        })
    }
}

/// Create a new session after verifying the agent exists and resolving the provider profile.
pub async fn create_session(
    state: &AppState,
    req: CreateSessionRequest,
) -> Result<Session, SessionError> {
    let CreateSessionRequest {
        agent_id,
        provider_profile_id,
        title,
        speedwagon_ids,
        source_ids,
    } = req;

    // Verify agent exists
    match state.repository.get_agent(agent_id).await? {
        Some(_) => {}
        None => return Err(SessionError::AgentNotFound),
    }

    // Resolve provider profile
    let resolved_id = resolve_provider_profile_id(state, provider_profile_id).await?;

    let session = state
        .repository
        .create_session(agent_id, resolved_id, title, speedwagon_ids, source_ids)
        .await?;

    Ok(session)
}

/// Update a session atomically (title, provider, speedwagons, sources) and invalidate runtime cache.
pub async fn update_session(
    state: &AppState,
    id: Uuid,
    req: UpdateSessionRequest,
) -> Result<Session, SessionError> {
    // Validate provider profile if provided
    if let Some(pp_id) = req.provider_profile_id {
        match state.repository.get_provider_profile(pp_id).await? {
            Some(_) => {}
            None => return Err(SessionError::ProviderProfileNotFound),
        }
    }

    // Atomic update via transaction
    let session = state
        .repository
        .update_session_atomic(
            id,
            req.title,
            req.provider_profile_id,
            req.speedwagon_ids,
            req.source_ids,
        )
        .await?
        .ok_or(SessionError::NotFound)?;

    // Invalidate cache once after successful commit
    state.invalidate_session_runtime(id);

    Ok(session)
}

/// Delete a session and invalidate its runtime cache.
pub async fn delete_session(state: &AppState, id: Uuid) -> Result<(), SessionError> {
    let deleted = state.repository.delete_session(id).await?;
    if !deleted {
        return Err(SessionError::NotFound);
    }
    state.invalidate_session_runtime(id);
    Ok(())
}

/// Save user message, run ChatAgent, and save assistant response.
pub async fn send_message(
    state: &AppState,
    session_id: Uuid,
    role: MessageRole,
    content: String,
) -> Result<AddSessionMessageResponse, SessionError> {
    if content.trim().is_empty() {
        return Err(SessionError::EmptyContent);
    }

    // Save the user message
    state
        .repository
        .add_session_message(session_id, role.clone(), content.clone())
        .await?
        .ok_or(SessionError::NotFound)?;

    if !matches!(role, MessageRole::User) {
        return Ok(AddSessionMessageResponse {
            assistant_message: None,
        });
    }

    // Load session once for runtime creation (without full message history reload per message)
    let session = state
        .repository
        .get_session(session_id)
        .await?
        .ok_or(SessionError::NotFound)?;

    let runtime = state
        .get_or_create_runtime_for_session(&session)
        .await
        .map_err(|e| SessionError::Runtime(format!("failed to initialize agent runtime: {e}")))?;

    let runtime_output = {
        let mut rt = runtime.lock().await;
        rt.run_user_text(content).await
    };

    let assistant_text = match runtime_output {
        Ok(text) => text,
        Err(ChatAgentRunError::NoTextContent) => {
            return Err(SessionError::Runtime(
                "model response did not include text content".to_string(),
            ));
        }
        Err(ChatAgentRunError::Runtime { source }) => {
            eprintln!("runtime execution error: {source}");
            return Err(SessionError::Runtime(
                "failed to run language model".to_string(),
            ));
        }
    };

    // Save assistant message and return it directly (no full session reload)
    let assistant_message = state
        .repository
        .add_session_message(session_id, MessageRole::Assistant, assistant_text)
        .await?;

    Ok(AddSessionMessageResponse { assistant_message })
}

async fn resolve_provider_profile_id(
    state: &AppState,
    requested: Option<Uuid>,
) -> Result<Uuid, SessionError> {
    if let Some(id) = requested {
        return match state.repository.get_provider_profile(id).await? {
            Some(_) => Ok(id),
            None => Err(SessionError::ProviderProfileNotFound),
        };
    }

    let profiles = state.repository.list_provider_profiles().await?;

    profiles
        .iter()
        .filter(|p| p.is_default)
        .min_by(|a, b| {
            provider_priority(&a.provider)
                .cmp(&provider_priority(&b.provider))
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.id.cmp(&b.id))
        })
        .map(|p| p.id)
        .ok_or(SessionError::NoDefaultProviderProfile)
}

fn provider_priority(provider: &AgentProvider) -> u8 {
    match &provider.lm {
        LangModelProvider::API { schema, .. } => match schema {
            LangModelAPISchema::ChatCompletion => 0,
            LangModelAPISchema::Anthropic => 1,
            LangModelAPISchema::Gemini => 2,
            LangModelAPISchema::OpenAI => 3,
        },
    }
}
