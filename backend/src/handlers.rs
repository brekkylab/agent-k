use std::sync::Arc;

use actix_multipart::Multipart;
use actix_web::http::StatusCode;
use actix_web::{HttpResponse, web};
use futures_util::StreamExt;
use ailoy::{AgentProvider, LangModelAPISchema, LangModelProvider};
use chat_agent::ChatAgentRunError;
use serde::Serialize;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use crate::agent::spec::{
    AgentProvider as ApiAgentProvider, LangModelProvider as ApiLangModelProvider,
};
use crate::models::{
    AddSessionMessageRequest, AddSessionMessageResponse, Agent, AgentResponse,
    CreateAgentRequest, CreateProviderProfileRequest,
    CreateSessionRequest, CreateSpeedwagonRequest, ErrorResponse, ListSessionsQuery, MessageRole,
    ProviderProfile, ProviderProfileResponse, Session, SessionMessage, SourceResponse, SourceType,
    Speedwagon, SpeedwagonIndexStatus, SpeedwagonResponse,
    UpdateAgentRequest, UpdateProviderProfileRequest, UpdateSessionRequest, UpdateSpeedwagonRequest,
};
use crate::repository::RepositoryError;
use crate::state::AppState;

#[derive(Serialize, ToSchema)]
struct HealthResponse {
    status: &'static str,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        create_agent,
        list_agents,
        get_agent,
        update_agent,
        delete_agent,
        create_provider_profile,
        list_provider_profiles,
        get_provider_profile,
        update_provider_profile,
        delete_provider_profile,
        create_session,
        list_sessions,
        get_session,
        update_session,
        delete_session,
        add_message,
        upload_source,
        list_sources,
        get_source,
        delete_source,
        create_speedwagon,
        list_speedwagons,
        get_speedwagon,
        update_speedwagon,
        delete_speedwagon,
        index_speedwagon,
    ),
    components(
        schemas(
            HealthResponse,
            ErrorResponse,
            AgentResponse,
            CreateAgentRequest,
            UpdateAgentRequest,
            ProviderProfileResponse,
            CreateProviderProfileRequest,
            UpdateProviderProfileRequest,
            Session,
            SessionMessage,
            MessageRole,
            CreateSessionRequest,
            UpdateSessionRequest,
            AddSessionMessageRequest,
            AddSessionMessageResponse,
            SourceResponse,
            SourceType,
            SpeedwagonResponse,
            SpeedwagonIndexStatus,
            CreateSpeedwagonRequest,
            UpdateSpeedwagonRequest,
            crate::agent::spec::AgentSpec,
            crate::agent::spec::LangModelAPISchema,
            crate::agent::spec::LangModelProvider,
            crate::agent::spec::MCPToolProvider,
            crate::agent::spec::ToolProvider,
            crate::agent::spec::AgentProvider
        )
    ),
    tags(
        (name = "system", description = "System endpoints"),
        (name = "agents", description = "Agent endpoints"),
        (name = "provider_profiles", description = "Provider profile endpoints"),
        (name = "sessions", description = "Session endpoints"),
        (name = "sources", description = "Source endpoints"),
        (name = "speedwagons", description = "Speedwagon endpoints")
    )
)]
pub struct ApiDoc;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/health").route(web::get().to(health)))
        .service(
            web::resource("/agents")
                .route(web::get().to(list_agents))
                .route(web::post().to(create_agent)),
        )
        .service(
            web::resource("/agents/{id}")
                .route(web::get().to(get_agent))
                .route(web::put().to(update_agent))
                .route(web::delete().to(delete_agent)),
        )
        .service(
            web::resource("/provider-profiles")
                .route(web::get().to(list_provider_profiles))
                .route(web::post().to(create_provider_profile)),
        )
        .service(
            web::resource("/provider-profiles/{id}")
                .route(web::get().to(get_provider_profile))
                .route(web::put().to(update_provider_profile))
                .route(web::delete().to(delete_provider_profile)),
        )
        .service(
            web::resource("/sessions")
                .route(web::get().to(list_sessions))
                .route(web::post().to(create_session)),
        )
        .service(
            web::resource("/sessions/{id}")
                .route(web::get().to(get_session))
                .route(web::put().to(update_session))
                .route(web::delete().to(delete_session)),
        )
        .service(web::resource("/sessions/{id}/messages").route(web::post().to(add_message)))
        .service(
            web::resource("/sources")
                .route(web::get().to(list_sources))
                .route(web::post().to(upload_source)),
        )
        .service(
            web::resource("/sources/{id}")
                .route(web::get().to(get_source))
                .route(web::delete().to(delete_source)),
        )
        .service(
            web::resource("/speedwagons")
                .route(web::get().to(list_speedwagons))
                .route(web::post().to(create_speedwagon)),
        )
        .service(
            web::resource("/speedwagons/{id}")
                .route(web::get().to(get_speedwagon))
                .route(web::put().to(update_speedwagon))
                .route(web::delete().to(delete_speedwagon)),
        )
        .service(
            web::resource("/speedwagons/{id}/index")
                .route(web::post().to(index_speedwagon)),
        );
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses((status = 200, description = "Health check", body = HealthResponse))
)]
async fn health() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse { status: "ok" })
}

#[utoipa::path(
    post,
    path = "/agents",
    tag = "agents",
    request_body = CreateAgentRequest,
    responses((status = 201, description = "Created agent", body = AgentResponse))
)]
async fn create_agent(
    state: web::Data<AppState>,
    payload: web::Json<CreateAgentRequest>,
) -> HttpResponse {
    let CreateAgentRequest { spec } = payload.into_inner();

    match state.repository.create_agent(spec.into()).await {
        Ok(agent) => HttpResponse::Created().json(to_agent_response(&agent)),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/agents",
    tag = "agents",
    responses((status = 200, description = "List agents", body = [AgentResponse]))
)]
async fn list_agents(state: web::Data<AppState>) -> HttpResponse {
    match state.repository.list_agents().await {
        Ok(agents) => {
            let response: Vec<AgentResponse> = agents.iter().map(to_agent_response).collect();
            HttpResponse::Ok().json(response)
        }
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/agents/{id}",
    tag = "agents",
    params(("id" = Uuid, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Agent", body = AgentResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    )
)]
async fn get_agent(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    match state.repository.get_agent(id).await {
        Ok(Some(agent)) => HttpResponse::Ok().json(to_agent_response(&agent)),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "agent not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/agents/{id}",
    tag = "agents",
    params(("id" = Uuid, Path, description = "Agent ID")),
    request_body = UpdateAgentRequest,
    responses(
        (status = 200, description = "Updated agent", body = AgentResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    )
)]
async fn update_agent(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
    payload: web::Json<UpdateAgentRequest>,
) -> HttpResponse {
    let id = path.into_inner();
    let UpdateAgentRequest { spec } = payload.into_inner();

    match state.repository.update_agent(id, spec.into()).await {
        Ok(Some(agent)) => {
            state.invalidate_runtimes_by_agent_id(id);
            HttpResponse::Ok().json(to_agent_response(&agent))
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, "agent not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/agents/{id}",
    tag = "agents",
    params(("id" = Uuid, Path, description = "Agent ID")),
    responses(
        (status = 204, description = "Agent deleted"),
        (status = 404, description = "Agent not found", body = ErrorResponse),
        (
            status = 409,
            description = "Agent has existing sessions",
            body = ErrorResponse
        )
    )
)]
async fn delete_agent(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    match state.repository.has_sessions_for_agent(id).await {
        Ok(true) => {
            return json_error(
                StatusCode::CONFLICT,
                "cannot delete agent with existing sessions",
            );
        }
        Ok(false) => {}
        Err(error) => return repository_error_response(error),
    }

    match state.repository.delete_agent(id).await {
        Ok(true) => HttpResponse::NoContent().finish(),
        Ok(false) => json_error(StatusCode::NOT_FOUND, "agent not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/provider-profiles",
    tag = "provider_profiles",
    request_body = CreateProviderProfileRequest,
    responses((status = 201, description = "Created provider profile", body = ProviderProfileResponse))
)]
async fn create_provider_profile(
    state: web::Data<AppState>,
    payload: web::Json<CreateProviderProfileRequest>,
) -> HttpResponse {
    let CreateProviderProfileRequest {
        name,
        provider,
        is_default,
    } = payload.into_inner();

    if name.trim().is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "provider profile name is empty");
    }

    match state
        .repository
        .create_provider_profile(name, provider.into(), is_default)
        .await
    {
        Ok(profile) => HttpResponse::Created().json(to_provider_profile_response(&profile)),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/provider-profiles",
    tag = "provider_profiles",
    responses((status = 200, description = "List provider profiles", body = [ProviderProfileResponse]))
)]
async fn list_provider_profiles(state: web::Data<AppState>) -> HttpResponse {
    match state.repository.list_provider_profiles().await {
        Ok(profiles) => {
            let response: Vec<ProviderProfileResponse> =
                profiles.iter().map(to_provider_profile_response).collect();
            HttpResponse::Ok().json(response)
        }
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/provider-profiles/{id}",
    tag = "provider_profiles",
    params(("id" = Uuid, Path, description = "Provider profile ID")),
    responses(
        (status = 200, description = "Provider profile", body = ProviderProfileResponse),
        (status = 404, description = "Provider profile not found", body = ErrorResponse)
    )
)]
async fn get_provider_profile(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    match state.repository.get_provider_profile(id).await {
        Ok(Some(profile)) => HttpResponse::Ok().json(to_provider_profile_response(&profile)),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "provider profile not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/provider-profiles/{id}",
    tag = "provider_profiles",
    params(("id" = Uuid, Path, description = "Provider profile ID")),
    request_body = UpdateProviderProfileRequest,
    responses(
        (status = 200, description = "Updated provider profile", body = ProviderProfileResponse),
        (status = 404, description = "Provider profile not found", body = ErrorResponse)
    )
)]
async fn update_provider_profile(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
    payload: web::Json<UpdateProviderProfileRequest>,
) -> HttpResponse {
    let id = path.into_inner();
    let UpdateProviderProfileRequest {
        name,
        provider,
        is_default,
    } = payload.into_inner();

    if name.trim().is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "provider profile name is empty");
    }

    match state
        .repository
        .update_provider_profile(id, name, provider.into(), is_default)
        .await
    {
        Ok(Some(profile)) => {
            state.invalidate_runtimes_by_provider_profile_id(id);
            HttpResponse::Ok().json(to_provider_profile_response(&profile))
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, "provider profile not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/provider-profiles/{id}",
    tag = "provider_profiles",
    params(("id" = Uuid, Path, description = "Provider profile ID")),
    responses(
        (status = 204, description = "Provider profile deleted"),
        (status = 404, description = "Provider profile not found", body = ErrorResponse),
        (
            status = 409,
            description = "Provider profile has existing sessions",
            body = ErrorResponse
        )
    )
)]
async fn delete_provider_profile(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
) -> HttpResponse {
    let id = path.into_inner();

    match state.repository.has_sessions_for_provider_profile(id).await {
        Ok(true) => {
            return json_error(
                StatusCode::CONFLICT,
                "cannot delete provider profile with existing sessions",
            );
        }
        Ok(false) => {}
        Err(error) => return repository_error_response(error),
    }

    match state.repository.delete_provider_profile(id).await {
        Ok(true) => HttpResponse::NoContent().finish(),
        Ok(false) => json_error(StatusCode::NOT_FOUND, "provider profile not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/sessions",
    tag = "sessions",
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Created session", body = Session),
        (status = 400, description = "No default provider profile", body = ErrorResponse),
        (status = 404, description = "Agent or provider profile not found", body = ErrorResponse)
    )
)]
async fn create_session(
    state: web::Data<AppState>,
    payload: web::Json<CreateSessionRequest>,
) -> HttpResponse {
    let CreateSessionRequest {
        agent_id,
        provider_profile_id,
        title,
        speedwagon_ids,
        source_ids,
    } = payload.into_inner();

    match state.repository.get_agent(agent_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "agent not found"),
        Err(error) => return repository_error_response(error),
    }

    let resolved_provider_profile_id =
        match resolve_provider_profile_id(&state, provider_profile_id).await {
            Ok(provider_profile_id) => provider_profile_id,
            Err(response) => return response,
        };

    match state
        .repository
        .create_session(agent_id, resolved_provider_profile_id, title, speedwagon_ids, source_ids)
        .await
    {
        Ok(session) => HttpResponse::Created().json(session),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/sessions",
    tag = "sessions",
    params(("agent_id" = Option<Uuid>, Query, description = "Filter by agent ID")),
    responses((status = 200, description = "List sessions", body = [Session]))
)]
async fn list_sessions(
    state: web::Data<AppState>,
    query: web::Query<ListSessionsQuery>,
) -> HttpResponse {
    let ListSessionsQuery {
        agent_id,
        include_messages,
    } = query.into_inner();

    match state
        .repository
        .list_sessions(agent_id, include_messages.unwrap_or(false))
        .await
    {
        Ok(sessions) => HttpResponse::Ok().json(sessions),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/sessions/{id}",
    tag = "sessions",
    params(("id" = Uuid, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Session", body = Session),
        (status = 404, description = "Session not found", body = ErrorResponse)
    )
)]
async fn get_session(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    match state.repository.get_session(id).await {
        Ok(Some(session)) => HttpResponse::Ok().json(session),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "session not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/sessions/{id}",
    tag = "sessions",
    params(("id" = Uuid, Path, description = "Session ID")),
    request_body = UpdateSessionRequest,
    responses(
        (status = 200, description = "Updated session", body = Session),
        (status = 404, description = "Session not found", body = ErrorResponse)
    )
)]
async fn update_session(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
    payload: web::Json<UpdateSessionRequest>,
) -> HttpResponse {
    let id = path.into_inner();
    let req = payload.into_inner();

    // Verify session exists first
    match state.repository.get_session(id).await {
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "session not found"),
        Err(error) => return repository_error_response(error),
        Ok(Some(_)) => {}
    }

    if let Some(title) = req.title {
        if let Err(error) = state.repository.update_session_title(id, title).await {
            return repository_error_response(error);
        }
    }

    if let Some(provider_profile_id) = req.provider_profile_id {
        // Verify provider profile exists
        match state.repository.get_provider_profile(provider_profile_id).await {
            Ok(None) => {
                return json_error(StatusCode::NOT_FOUND, "provider profile not found");
            }
            Err(error) => return repository_error_response(error),
            Ok(Some(_)) => {}
        }

        if let Err(error) = state
            .repository
            .update_session_provider_profile_id(id, provider_profile_id)
            .await
        {
            return repository_error_response(error);
        }
        state.invalidate_session_runtime(id);
    }

    if let Some(speedwagon_ids) = req.speedwagon_ids {
        if let Err(error) = state
            .repository
            .set_session_speedwagons(id, speedwagon_ids)
            .await
        {
            return repository_error_response(error);
        }
        state.invalidate_session_runtime(id);
    }

    if let Some(source_ids) = req.source_ids {
        if let Err(error) = state
            .repository
            .set_session_sources(id, source_ids)
            .await
        {
            return repository_error_response(error);
        }
        state.invalidate_session_runtime(id);
    }

    match state.repository.get_session(id).await {
        Ok(Some(session)) => HttpResponse::Ok().json(session),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "session not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/sessions/{id}",
    tag = "sessions",
    params(("id" = Uuid, Path, description = "Session ID")),
    responses(
        (status = 204, description = "Session deleted"),
        (status = 404, description = "Session not found", body = ErrorResponse)
    )
)]
async fn delete_session(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    match state.repository.delete_session(id).await {
        Ok(true) => {
            state.invalidate_session_runtime(id);
            HttpResponse::NoContent().finish()
        }
        Ok(false) => json_error(StatusCode::NOT_FOUND, "session not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/sessions/{id}/messages",
    tag = "sessions",
    params(("id" = Uuid, Path, description = "Session ID")),
    request_body = AddSessionMessageRequest,
    responses(
        (status = 200, description = "Assistant output", body = AddSessionMessageResponse),
        (status = 400, description = "Empty message content", body = ErrorResponse),
        (status = 404, description = "Session not found", body = ErrorResponse),
        (status = 502, description = "Runtime/provider failure", body = ErrorResponse)
    )
)]
async fn add_message(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
    payload: web::Json<AddSessionMessageRequest>,
) -> HttpResponse {
    let id = path.into_inner();
    let AddSessionMessageRequest { role, content } = payload.into_inner();

    if content.trim().is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "message content is empty");
    }

    let session = match state
        .repository
        .add_session_message(id, role.clone(), content.clone())
        .await
    {
        Ok(Some(session)) => session,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "session not found"),
        Err(error) => return repository_error_response(error),
    };

    if !matches!(role, MessageRole::User) {
        return HttpResponse::Ok().json(AddSessionMessageResponse {
            assistant_message: None,
        });
    }

    let runtime = match state.get_or_create_runtime_for_session(&session).await {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("runtime initialization error: {error}");
            return json_error(
                StatusCode::BAD_GATEWAY,
                "failed to initialize agent runtime",
            );
        }
    };

    let runtime_output = {
        let mut runtime = runtime.lock().await;
        runtime.run_user_text(content).await
    };

    let assistant_text = match runtime_output {
        Ok(text) => text,
        Err(ChatAgentRunError::NoTextContent) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                "model response did not include text content",
            );
        }
        Err(ChatAgentRunError::Runtime { source }) => {
            eprintln!("runtime execution error: {source}");
            return json_error(StatusCode::BAD_GATEWAY, "failed to run language model");
        }
    };

    match state
        .repository
        .add_session_message(id, MessageRole::Assistant, assistant_text)
        .await
    {
        Ok(Some(updated_session)) => {
            let assistant_message = updated_session
                .messages
                .last()
                .cloned()
                .filter(|message| matches!(message.role, MessageRole::Assistant));

            HttpResponse::Ok().json(AddSessionMessageResponse { assistant_message })
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, "session not found"),
        Err(error) => repository_error_response(error),
    }
}

// ===================== Source Handlers =====================

#[utoipa::path(
    post,
    path = "/sources",
    tag = "sources",
    responses(
        (status = 201, description = "Uploaded source", body = SourceResponse),
        (status = 400, description = "No file in request", body = ErrorResponse)
    )
)]
async fn upload_source(
    state: web::Data<AppState>,
    mut payload: Multipart,
) -> HttpResponse {
    // Extract the first file field from the multipart stream
    let mut file_name = String::new();
    let mut file_bytes: Vec<u8> = Vec::new();

    while let Some(Ok(mut field)) = payload.next().await {
        if let Some(disposition) = field.content_disposition() {
            if let Some(name) = disposition.get_filename() {
                file_name = name.to_string();
            }
        }
        while let Some(Ok(chunk)) = field.next().await {
            file_bytes.extend_from_slice(&chunk);
        }
        // Only handle the first file
        break;
    }

    if file_name.is_empty() || file_bytes.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "no file in request");
    }

    let size = file_bytes.len() as i64;

    // Construct stored filename: {original}-{timestamp}.{ext}
    let timestamp = chrono::Utc::now().timestamp_millis();
    let stored_name = if let Some(dot_pos) = file_name.rfind('.') {
        let (stem, ext) = file_name.split_at(dot_pos);
        format!("{stem}-{timestamp}{ext}")
    } else {
        format!("{file_name}-{timestamp}")
    };

    let upload_dir = state.upload_dir.clone();
    if let Err(error) = tokio::fs::create_dir_all(&upload_dir).await {
        eprintln!("failed to create upload dir: {error}");
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "failed to create upload directory");
    }

    let file_path = upload_dir.join(&stored_name);
    if let Err(error) = tokio::fs::write(&file_path, &file_bytes).await {
        eprintln!("failed to write file: {error}");
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "failed to write file");
    }

    let file_path_str = file_path.to_string_lossy().to_string();

    match state
        .repository
        .create_source(file_name, SourceType::LocalFile, Some(file_path_str), size)
        .await
    {
        Ok(source) => HttpResponse::Created().json(SourceResponse::from(&source)),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/sources",
    tag = "sources",
    responses((status = 200, description = "List sources", body = [SourceResponse]))
)]
async fn list_sources(state: web::Data<AppState>) -> HttpResponse {
    match state.repository.list_sources().await {
        Ok(sources) => {
            let response: Vec<SourceResponse> = sources.iter().map(SourceResponse::from).collect();
            HttpResponse::Ok().json(response)
        }
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/sources/{id}",
    tag = "sources",
    params(("id" = Uuid, Path, description = "Source ID")),
    responses(
        (status = 200, description = "Source", body = SourceResponse),
        (status = 404, description = "Source not found", body = ErrorResponse)
    )
)]
async fn get_source(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    match state.repository.get_source(id).await {
        Ok(Some(source)) => HttpResponse::Ok().json(SourceResponse::from(&source)),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "source not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/sources/{id}",
    tag = "sources",
    params(("id" = Uuid, Path, description = "Source ID")),
    responses(
        (status = 204, description = "Source deleted"),
        (status = 404, description = "Source not found", body = ErrorResponse)
    )
)]
async fn delete_source(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    // Get file_path before deleting from DB
    let file_path = match state.repository.get_source(id).await {
        Ok(Some(source)) => source.file_path,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "source not found"),
        Err(error) => return repository_error_response(error),
    };

    match state.repository.delete_source(id).await {
        Ok(true) => {
            // Delete file from disk
            if let Some(path) = &file_path {
                if let Err(error) = tokio::fs::remove_file(path).await {
                    eprintln!("failed to delete file {path}: {error}");
                }
            }

            // Reset index_status for any speedwagons that referenced this source
            if let Ok(speedwagons) = state.repository.list_speedwagons().await {
                for sw in speedwagons {
                    if sw.source_ids.contains(&id) && sw.index_status != SpeedwagonIndexStatus::NotIndexed {
                        let _ = state.repository.update_speedwagon_index_status(
                            sw.id,
                            SpeedwagonIndexStatus::NotIndexed,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ).await;
                    }
                }
            }

            HttpResponse::NoContent().finish()
        }
        Ok(false) => json_error(StatusCode::NOT_FOUND, "source not found"),
        Err(error) => repository_error_response(error),
    }
}

// ===================== Speedwagon Handlers =====================

#[utoipa::path(
    post,
    path = "/speedwagons",
    tag = "speedwagons",
    request_body = CreateSpeedwagonRequest,
    responses((status = 201, description = "Created speedwagon", body = SpeedwagonResponse))
)]
async fn create_speedwagon(
    state: web::Data<AppState>,
    payload: web::Json<CreateSpeedwagonRequest>,
) -> HttpResponse {
    let CreateSpeedwagonRequest { name, description, instruction, lm, source_ids } = payload.into_inner();

    match state.repository.create_speedwagon(name, description, instruction, lm, source_ids).await {
        Ok(sw) => HttpResponse::Created().json(SpeedwagonResponse::from(&sw)),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/speedwagons",
    tag = "speedwagons",
    responses((status = 200, description = "List speedwagons", body = [SpeedwagonResponse]))
)]
async fn list_speedwagons(state: web::Data<AppState>) -> HttpResponse {
    match state.repository.list_speedwagons().await {
        Ok(list) => {
            let response: Vec<SpeedwagonResponse> = list.iter().map(SpeedwagonResponse::from).collect();
            HttpResponse::Ok().json(response)
        }
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/speedwagons/{id}",
    tag = "speedwagons",
    params(("id" = Uuid, Path, description = "Speedwagon ID")),
    responses(
        (status = 200, description = "Speedwagon", body = SpeedwagonResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    )
)]
async fn get_speedwagon(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();
    match state.repository.get_speedwagon(id).await {
        Ok(Some(sw)) => HttpResponse::Ok().json(SpeedwagonResponse::from(&sw)),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "speedwagon not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/speedwagons/{id}",
    tag = "speedwagons",
    params(("id" = Uuid, Path, description = "Speedwagon ID")),
    request_body = UpdateSpeedwagonRequest,
    responses(
        (status = 200, description = "Updated speedwagon", body = SpeedwagonResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    )
)]
async fn update_speedwagon(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
    payload: web::Json<UpdateSpeedwagonRequest>,
) -> HttpResponse {
    let id = path.into_inner();
    let UpdateSpeedwagonRequest { name, description, instruction, lm, source_ids } = payload.into_inner();

    match state.repository.update_speedwagon(id, name, description, instruction, lm, source_ids).await {
        Ok(Some(sw)) => HttpResponse::Ok().json(SpeedwagonResponse::from(&sw)),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "speedwagon not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/speedwagons/{id}",
    tag = "speedwagons",
    params(("id" = Uuid, Path, description = "Speedwagon ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = ErrorResponse)
    )
)]
async fn delete_speedwagon(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    // Get data_dir before deleting
    let sw = match state.repository.get_speedwagon(id).await {
        Ok(Some(sw)) => sw,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "speedwagon not found"),
        Err(error) => return repository_error_response(error),
    };

    match state.repository.delete_speedwagon(id).await {
        Ok(true) => {
            // Delete data/speedwagons/{id}/ directory from disk
            let dir = state.speedwagon_data_dir.join(sw.id.to_string());
            if dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    eprintln!("failed to delete speedwagon dir {}: {e}", dir.display());
                }
            }
            HttpResponse::NoContent().finish()
        }
        Ok(false) => json_error(StatusCode::NOT_FOUND, "speedwagon not found"),
        Err(error) => repository_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/speedwagons/{id}/index",
    tag = "speedwagons",
    params(("id" = Uuid, Path, description = "Speedwagon ID")),
    responses(
        (status = 202, description = "Indexing started"),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 409, description = "Already indexing", body = ErrorResponse),
        (status = 422, description = "No sources", body = ErrorResponse)
    )
)]
async fn index_speedwagon(state: web::Data<AppState>, path: web::Path<Uuid>) -> HttpResponse {
    let id = path.into_inner();

    let sw: Speedwagon = match state.repository.get_speedwagon(id).await {
        Ok(Some(sw)) => sw,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "speedwagon not found"),
        Err(error) => return repository_error_response(error),
    };

    if sw.source_ids.is_empty() {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "speedwagon has no sources");
    }

    // Concurrent indexing guard
    if sw.index_status == SpeedwagonIndexStatus::Indexing {
        return json_error(StatusCode::CONFLICT, "indexing already in progress");
    }

    // Set status to indexing
    let now = chrono::Utc::now();
    if let Err(error) = state
        .repository
        .update_speedwagon_index_status(
            id,
            SpeedwagonIndexStatus::Indexing,
            None,
            None,
            None,
            Some(now),
            None,
        )
        .await
    {
        return repository_error_response(error);
    }

    // Clone everything needed for the background task
    let repository = Arc::clone(&state.repository);
    let speedwagon_data_dir = state.speedwagon_data_dir.clone();
    let state_clone = state.clone();

    tokio::task::spawn(async move {
        let _ = crate::services::indexing::start_indexing(
            repository,
            state_clone,
            speedwagon_data_dir,
            sw,
        )
        .await;
    });

    HttpResponse::Accepted().finish()
}

async fn resolve_provider_profile_id(
    state: &web::Data<AppState>,
    requested_provider_profile_id: Option<Uuid>,
) -> Result<Uuid, HttpResponse> {
    if let Some(provider_profile_id) = requested_provider_profile_id {
        return match state
            .repository
            .get_provider_profile(provider_profile_id)
            .await
        {
            Ok(Some(_)) => Ok(provider_profile_id),
            Ok(None) => Err(json_error(
                StatusCode::NOT_FOUND,
                "provider profile not found",
            )),
            Err(error) => Err(repository_error_response(error)),
        };
    }

    let profiles = state
        .repository
        .list_provider_profiles()
        .await
        .map_err(repository_error_response)?;

    let Some(profile) = profiles
        .iter()
        .filter(|profile| profile.is_default)
        .min_by(|a, b| {
            provider_priority(&a.provider)
                .cmp(&provider_priority(&b.provider))
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.id.cmp(&b.id))
        })
    else {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "no default provider profile available",
        ));
    };

    Ok(profile.id)
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

fn to_agent_response(agent: &Agent) -> AgentResponse {
    AgentResponse {
        id: agent.id,
        spec: agent.spec.clone().into(),
        created_at: agent.created_at,
        updated_at: agent.updated_at,
    }
}

fn to_provider_profile_response(profile: &ProviderProfile) -> ProviderProfileResponse {
    let mut provider: ApiAgentProvider = profile.provider.clone().into();
    let ApiLangModelProvider::API { api_key, .. } = &mut provider.lm;
    *api_key = None;

    ProviderProfileResponse {
        id: profile.id,
        name: profile.name.clone(),
        provider,
        is_default: profile.is_default,
        created_at: profile.created_at,
        updated_at: profile.updated_at,
    }
}

fn repository_error_response(error: RepositoryError) -> HttpResponse {
    if let RepositoryError::Database(sqlx::Error::Database(db_error)) = &error {
        let msg = db_error.message();
        if msg.contains("UNIQUE constraint failed: provider_profiles.name") {
            return json_error(StatusCode::CONFLICT, "provider profile name already exists");
        }
    }

    eprintln!("repository error: {error}");
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}

fn json_error(status: StatusCode, error: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(ErrorResponse {
        error: error.into(),
    })
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use actix_web::{App, HttpResponse, HttpServer, dev::Service, http::StatusCode, test, web};
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::configure;
    use crate::state::AppState;

    fn provider_payload(schema: &str, url: &str, key: &str) -> Value {
        json!({
            "lm": {
                "type": "api",
                "schema": schema,
                "url": url,
                "api_key": key
            },
            "tools": []
        })
    }

    fn test_database_url(temp_dir: &TempDir) -> String {
        format!("sqlite://{}", temp_dir.path().join("app.db").display())
    }

    async fn test_state_with_upload_dir(temp_dir: &TempDir) -> web::Data<AppState> {
        let database_url = test_database_url(temp_dir);
        let upload_dir = temp_dir.path().join("uploads");
        web::Data::new(
            AppState::new_without_bootstrap_with_upload_dir(&database_url, upload_dir)
                .await
                .expect("state should be created"),
        )
    }

    async fn create_agent(
        app: &impl Service<
            actix_http::Request,
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
        >,
    ) -> Uuid {
        let req = test::TestRequest::post()
            .uri("/agents")
            .set_json(json!({
                "spec": {
                    "lm": "gpt-4.1",
                    "instruction": null,
                    "tools": []
                }
            }))
            .to_request();
        let body: Value = test::call_and_read_body_json(app, req).await;
        Uuid::parse_str(body["id"].as_str().expect("agent id must exist"))
            .expect("agent id must be uuid")
    }

    async fn create_provider_profile(
        app: &impl Service<
            actix_http::Request,
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
        >,
        name: &str,
        schema: &str,
        is_default: bool,
    ) -> Uuid {
        create_provider_profile_with_url(app, name, schema, "https://example.com/v1", is_default)
            .await
    }

    async fn create_provider_profile_with_url(
        app: &impl Service<
            actix_http::Request,
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
        >,
        name: &str,
        schema: &str,
        url: &str,
        is_default: bool,
    ) -> Uuid {
        let req = test::TestRequest::post()
            .uri("/provider-profiles")
            .set_json(json!({
                "name": name,
                "provider": provider_payload(schema, url, "secret-key"),
                "is_default": is_default
            }))
            .to_request();
        let body: Value = test::call_and_read_body_json(app, req).await;
        Uuid::parse_str(body["id"].as_str().expect("provider profile id must exist"))
            .expect("provider profile id must be uuid")
    }

    async fn start_mock_chat_completion_server()
    -> (String, Arc<Mutex<Vec<usize>>>, actix_web::dev::ServerHandle) {
        let request_message_counts = Arc::new(Mutex::new(Vec::new()));
        let counts_state = Arc::clone(&request_message_counts);

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind should work");
        let addr = listener
            .local_addr()
            .expect("local addr should be available");

        let server = HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(Arc::clone(&counts_state)))
                .route(
                    "/v1/chat/completions",
                    web::post().to(
                        |counts: web::Data<Arc<Mutex<Vec<usize>>>>, body: web::Json<Value>| async move {
                            let message_count = body
                                .get("messages")
                                .and_then(Value::as_array)
                                .map(|messages| messages.len())
                                .unwrap_or(0);

                            counts
                                .lock()
                                .expect("message count lock should be available")
                                .push(message_count);

                            let last_user_text = body
                                .pointer("/messages")
                                .and_then(Value::as_array)
                                .and_then(|messages| messages.last())
                                .and_then(|message| {
                                    message
                                        .pointer("/content/0/text")
                                        .and_then(Value::as_str)
                                        .or_else(|| message.pointer("/content").and_then(Value::as_str))
                                })
                                .unwrap_or("ok");

                            HttpResponse::Ok().json(json!({
                                "choices": [
                                    {
                                        "finish_reason": "stop",
                                        "message": {
                                            "role": "assistant",
                                            "content": [
                                                {
                                                    "type": "text",
                                                    "text": format!("assistant:{last_user_text}")
                                                }
                                            ]
                                        }
                                    }
                                ]
                            }))
                        },
                    ),
                )
        })
        .listen(listener)
        .expect("server should listen")
        .run();

        let handle = server.handle();
        actix_web::rt::spawn(server);

        (
            format!("http://{addr}/v1/chat/completions"),
            request_message_counts,
            handle,
        )
    }

    #[actix_web::test]
    async fn create_agent_rejects_provider_field() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let req = test::TestRequest::post()
            .uri("/agents")
            .set_json(json!({
                "spec": {
                    "lm": "gpt-4.1",
                    "instruction": null,
                    "tools": []
                },
                "provider": {
                    "lm": {
                        "type": "api",
                        "schema": "chat_completion",
                        "url": "https://api.openai.com/v1/chat/completions",
                        "api_key": "secret"
                    },
                    "tools": []
                }
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn provider_profile_api_hides_api_key() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let create_req = test::TestRequest::post()
            .uri("/provider-profiles")
            .set_json(json!({
                "name": "openai-default",
                "provider": provider_payload(
                    "chat_completion",
                    "https://api.openai.com/v1/chat/completions",
                    "very-secret"
                ),
                "is_default": true
            }))
            .to_request();
        let create_body: Value = test::call_and_read_body_json(&app, create_req).await;
        assert!(create_body["provider"]["lm"]["api_key"].is_null());

        let profile_id = create_body["id"].as_str().expect("profile id must exist");

        let get_req = test::TestRequest::get()
            .uri(&format!("/provider-profiles/{profile_id}"))
            .to_request();
        let get_body: Value = test::call_and_read_body_json(&app, get_req).await;
        assert!(get_body["provider"]["lm"]["api_key"].is_null());

        let list_req = test::TestRequest::get()
            .uri("/provider-profiles")
            .to_request();
        let list_body: Value = test::call_and_read_body_json(&app, list_req).await;
        assert!(
            list_body.as_array().expect("list should be array")[0]["provider"]["lm"]["api_key"]
                .is_null()
        );
    }

    #[actix_web::test]
    async fn create_session_with_explicit_profile_or_missing_profile() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }))
            .to_request();
        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        assert_eq!(
            session_body["provider_profile_id"],
            Value::String(profile_id.to_string())
        );

        let missing_profile_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": Uuid::new_v4()
            }))
            .to_request();
        let resp = test::call_service(&app, missing_profile_req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn session_default_profile_selection_follows_priority() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;

        let _gemini_id = create_provider_profile(&app, "gemini-default", "gemini", true).await;
        let _anthropic_id =
            create_provider_profile(&app, "anthropic-default", "anthropic", true).await;
        let openai_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id
            }))
            .to_request();

        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        assert_eq!(
            session_body["provider_profile_id"],
            Value::String(openai_id.to_string())
        );
    }

    #[actix_web::test]
    async fn session_default_profile_tiebreak_uses_created_order() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let first = create_provider_profile(&app, "openai-first", "chat_completion", true).await;
        actix_web::rt::time::sleep(Duration::from_millis(2)).await;
        let _second = create_provider_profile(&app, "openai-second", "chat_completion", true).await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id
            }))
            .to_request();

        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        assert_eq!(
            session_body["provider_profile_id"],
            Value::String(first.to_string())
        );
    }

    #[actix_web::test]
    async fn create_session_without_default_profile_returns_bad_request() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let _profile_id =
            create_provider_profile(&app, "not-default", "chat_completion", false).await;

        let req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id
            }))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn delete_provider_profile_in_use_returns_conflict() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }))
            .to_request();
        let _session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;

        let delete_profile_req = test::TestRequest::delete()
            .uri(&format!("/provider-profiles/{profile_id}"))
            .to_request();
        let resp = test::call_service(&app, delete_profile_req).await;
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[actix_web::test]
    async fn add_message_non_user_role_does_not_trigger_inference() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }))
            .to_request();
        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let add_message_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "system", "content": "meta note" }))
            .to_request();
        let response_body: Value = test::call_and_read_body_json(&app, add_message_req).await;
        assert!(response_body["assistant_message"].is_null());

        let get_session_req = test::TestRequest::get()
            .uri(&format!("/sessions/{session_id}"))
            .to_request();
        let updated_session: Value = test::call_and_read_body_json(&app, get_session_req).await;
        let messages = updated_session["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], Value::String("system".to_string()));
    }

    #[actix_web::test]
    async fn add_message_runtime_failure_returns_bad_gateway_and_keeps_user_message() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let profile_id = create_provider_profile_with_url(
            &app,
            "openai-default",
            "chat_completion",
            "http://127.0.0.1:1/v1/chat/completions",
            true,
        )
        .await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }))
            .to_request();
        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let add_message_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "user", "content": "hello" }))
            .to_request();
        let response = test::call_service(&app, add_message_req).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let get_session_req = test::TestRequest::get()
            .uri(&format!("/sessions/{session_id}"))
            .to_request();
        let updated_session: Value = test::call_and_read_body_json(&app, get_session_req).await;
        let messages = updated_session["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], Value::String("user".to_string()));
        assert_eq!(messages[0]["content"], Value::String("hello".to_string()));
    }

    #[actix_web::test]
    async fn add_message_user_inference_uses_runtime_history_between_turns() {
        let (mock_url, request_counts, server_handle) = start_mock_chat_completion_server().await;
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let profile_id = create_provider_profile_with_url(
            &app,
            "openai-default",
            "chat_completion",
            &mock_url,
            true,
        )
        .await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }))
            .to_request();
        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let first_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "user", "content": "turn-1" }))
            .to_request();
        let first_body: Value = test::call_and_read_body_json(&app, first_req).await;
        assert_eq!(
            first_body["assistant_message"]["content"],
            Value::String("assistant:turn-1".to_string())
        );

        let second_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "user", "content": "turn-2" }))
            .to_request();
        let second_body: Value = test::call_and_read_body_json(&app, second_req).await;
        assert_eq!(
            second_body["assistant_message"]["content"],
            Value::String("assistant:turn-2".to_string())
        );

        let counts = request_counts
            .lock()
            .expect("request counts lock should be available")
            .clone();
        assert_eq!(counts, vec![1, 3]);

        server_handle.stop(true).await;
    }

    #[actix_web::test]
    async fn update_agent_resets_session_runtime_cache() {
        let (mock_url, request_counts, server_handle) = start_mock_chat_completion_server().await;
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let profile_id = create_provider_profile_with_url(
            &app,
            "openai-default",
            "chat_completion",
            &mock_url,
            true,
        )
        .await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }))
            .to_request();
        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let first_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "user", "content": "before-update" }))
            .to_request();
        let _first_body: Value = test::call_and_read_body_json(&app, first_req).await;

        let update_agent_req = test::TestRequest::put()
            .uri(&format!("/agents/{agent_id}"))
            .set_json(json!({
                "spec": {
                    "lm": "gpt-4.1-mini",
                    "instruction": null,
                    "tools": []
                }
            }))
            .to_request();
        let update_resp = test::call_service(&app, update_agent_req).await;
        assert_eq!(update_resp.status(), StatusCode::OK);

        let second_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "user", "content": "after-update" }))
            .to_request();
        let _second_body: Value = test::call_and_read_body_json(&app, second_req).await;

        let counts = request_counts
            .lock()
            .expect("request counts lock should be available")
            .clone();
        assert_eq!(counts, vec![1, 1]);

        server_handle.stop(true).await;
    }

    #[actix_web::test]
    async fn update_provider_profile_resets_session_runtime_cache() {
        let (mock_url, request_counts, server_handle) = start_mock_chat_completion_server().await;
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let profile_id = create_provider_profile_with_url(
            &app,
            "openai-default",
            "chat_completion",
            &mock_url,
            true,
        )
        .await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }))
            .to_request();
        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let first_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "user", "content": "before-provider-update" }))
            .to_request();
        let _first_body: Value = test::call_and_read_body_json(&app, first_req).await;

        let update_provider_req = test::TestRequest::put()
            .uri(&format!("/provider-profiles/{profile_id}"))
            .set_json(json!({
                "name": "openai-default",
                "provider": provider_payload("chat_completion", &mock_url, "another-secret"),
                "is_default": true
            }))
            .to_request();
        let update_resp = test::call_service(&app, update_provider_req).await;
        assert_eq!(update_resp.status(), StatusCode::OK);

        let second_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "user", "content": "after-provider-update" }))
            .to_request();
        let _second_body: Value = test::call_and_read_body_json(&app, second_req).await;

        let counts = request_counts
            .lock()
            .expect("request counts lock should be available")
            .clone();
        assert_eq!(counts, vec![1, 1]);

        server_handle.stop(true).await;
    }

    #[actix_web::test]
    async fn delete_session_removes_session_and_close_route_is_absent() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let create_session_req = test::TestRequest::post()
            .uri("/sessions")
            .set_json(json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }))
            .to_request();
        let session_body: Value = test::call_and_read_body_json(&app, create_session_req).await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let add_message_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/messages"))
            .set_json(json!({ "role": "system", "content": "hello" }))
            .to_request();
        let _response: Value = test::call_and_read_body_json(&app, add_message_req).await;

        let close_req = test::TestRequest::post()
            .uri(&format!("/sessions/{session_id}/close"))
            .to_request();
        let close_resp = test::call_service(&app, close_req).await;
        assert_eq!(close_resp.status(), StatusCode::NOT_FOUND);

        let delete_req = test::TestRequest::delete()
            .uri(&format!("/sessions/{session_id}"))
            .to_request();
        let delete_resp = test::call_service(&app, delete_req).await;
        assert_eq!(delete_resp.status(), StatusCode::NO_CONTENT);

        let get_req = test::TestRequest::get()
            .uri(&format!("/sessions/{session_id}"))
            .to_request();
        let get_resp = test::call_service(&app, get_req).await;
        assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);

        let delete_profile_req = test::TestRequest::delete()
            .uri(&format!("/provider-profiles/{profile_id}"))
            .to_request();
        let delete_profile_resp = test::call_service(&app, delete_profile_req).await;
        assert_eq!(delete_profile_resp.status(), StatusCode::NO_CONTENT);
    }

    // ===================== Source Tests =====================

    fn multipart_file_payload(filename: &str, content: &[u8]) -> (String, Vec<u8>) {
        let boundary = "----TestBoundary7MA4YWxkTrZu0gW";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (boundary.to_string(), body)
    }

    async fn upload_source(
        app: &impl Service<
            actix_http::Request,
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
        >,
        filename: &str,
        content: &[u8],
    ) -> Value {
        let (boundary, body) = multipart_file_payload(filename, content);
        let req = test::TestRequest::post()
            .uri("/sources")
            .insert_header((
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(body)
            .to_request();
        test::call_and_read_body_json(app, req).await
    }

    #[actix_web::test]
    async fn source_upload_and_list() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let state = test_state_with_upload_dir(&temp_dir).await;
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        // Upload a source
        let body = upload_source(&app, "test-doc.pdf", b"fake pdf content").await;
        assert_eq!(body["name"], Value::String("test-doc.pdf".to_string()));
        assert_eq!(body["source_type"], Value::String("local_file".to_string()));
        assert_eq!(body["size"], json!(16)); // b"fake pdf content".len()
        assert!(body["id"].as_str().is_some());
        // file_path must NOT be exposed in response
        assert!(body.get("file_path").is_none());

        let source_id = body["id"].as_str().expect("source id must exist").to_string();

        // List sources
        let list_req = test::TestRequest::get().uri("/sources").to_request();
        let list_body: Value = test::call_and_read_body_json(&app, list_req).await;
        let sources = list_body.as_array().expect("sources should be array");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0]["name"], Value::String("test-doc.pdf".to_string()));

        // Get source by ID
        let get_req = test::TestRequest::get()
            .uri(&format!("/sources/{source_id}"))
            .to_request();
        let get_body: Value = test::call_and_read_body_json(&app, get_req).await;
        assert_eq!(get_body["id"], Value::String(source_id.clone()));
        assert_eq!(get_body["name"], Value::String("test-doc.pdf".to_string()));

        // Delete source
        let delete_req = test::TestRequest::delete()
            .uri(&format!("/sources/{source_id}"))
            .to_request();
        let delete_resp = test::call_service(&app, delete_req).await;
        assert_eq!(delete_resp.status(), StatusCode::NO_CONTENT);

        // Verify deleted
        let get_after_delete = test::TestRequest::get()
            .uri(&format!("/sources/{source_id}"))
            .to_request();
        let resp = test::call_service(&app, get_after_delete).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn source_upload_no_file_returns_bad_request() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let state = test_state_with_upload_dir(&temp_dir).await;
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        // Send empty multipart
        let boundary = "----TestBoundary";
        let body = format!("--{boundary}--\r\n");
        let req = test::TestRequest::post()
            .uri("/sources")
            .insert_header((
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            ))
            .set_payload(body)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn source_delete_not_found() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let state = test_state_with_upload_dir(&temp_dir).await;
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let req = test::TestRequest::delete()
            .uri(&format!("/sources/{}", Uuid::new_v4()))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ===================== Knowledge Tests =====================

    #[actix_web::test]
    async fn knowledge_crud() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        // Create knowledge
        let create_req = test::TestRequest::post()
            .uri("/knowledges")
            .set_json(json!({
                "name": "마케팅 자료",
                "description": "마케팅 전략 관련 자료",
                "source_ids": []
            }))
            .to_request();
        let create_body: Value = test::call_and_read_body_json(&app, create_req).await;
        assert_eq!(create_body["name"], Value::String("마케팅 자료".to_string()));
        assert_eq!(
            create_body["description"],
            Value::String("마케팅 전략 관련 자료".to_string())
        );
        assert_eq!(create_body["source_ids"], json!([]));

        let knowledge_id = create_body["id"]
            .as_str()
            .expect("knowledge id must exist")
            .to_string();

        // List knowledges
        let list_req = test::TestRequest::get().uri("/knowledges").to_request();
        let list_body: Value = test::call_and_read_body_json(&app, list_req).await;
        let knowledges = list_body.as_array().expect("knowledges should be array");
        assert_eq!(knowledges.len(), 1);

        // Get knowledge
        let get_req = test::TestRequest::get()
            .uri(&format!("/knowledges/{knowledge_id}"))
            .to_request();
        let get_body: Value = test::call_and_read_body_json(&app, get_req).await;
        assert_eq!(get_body["name"], Value::String("마케팅 자료".to_string()));

        // Update knowledge
        let update_req = test::TestRequest::put()
            .uri(&format!("/knowledges/{knowledge_id}"))
            .set_json(json!({
                "name": "업데이트된 자료",
                "description": "새로운 설명",
                "source_ids": []
            }))
            .to_request();
        let update_body: Value = test::call_and_read_body_json(&app, update_req).await;
        assert_eq!(
            update_body["name"],
            Value::String("업데이트된 자료".to_string())
        );
        assert_eq!(
            update_body["description"],
            Value::String("새로운 설명".to_string())
        );

        // Delete knowledge
        let delete_req = test::TestRequest::delete()
            .uri(&format!("/knowledges/{knowledge_id}"))
            .to_request();
        let delete_resp = test::call_service(&app, delete_req).await;
        assert_eq!(delete_resp.status(), StatusCode::NO_CONTENT);

        // Verify deleted
        let get_after_delete = test::TestRequest::get()
            .uri(&format!("/knowledges/{knowledge_id}"))
            .to_request();
        let resp = test::call_service(&app, get_after_delete).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn knowledge_create_empty_name_returns_bad_request() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let req = test::TestRequest::post()
            .uri("/knowledges")
            .set_json(json!({
                "name": "  ",
                "description": "desc"
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn knowledge_with_source_ids() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let state = test_state_with_upload_dir(&temp_dir).await;
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        // Upload two sources
        let src1 = upload_source(&app, "doc1.pdf", b"content1").await;
        let src2 = upload_source(&app, "doc2.pdf", b"content2").await;
        let src1_id = src1["id"].as_str().expect("src1 id").to_string();
        let src2_id = src2["id"].as_str().expect("src2 id").to_string();

        // Create knowledge with source_ids
        let create_req = test::TestRequest::post()
            .uri("/knowledges")
            .set_json(json!({
                "name": "기술 문서",
                "description": "API 관련 자료",
                "source_ids": [src1_id, src2_id]
            }))
            .to_request();
        let create_body: Value = test::call_and_read_body_json(&app, create_req).await;
        let source_ids = create_body["source_ids"]
            .as_array()
            .expect("source_ids should be array");
        assert_eq!(source_ids.len(), 2);

        let knowledge_id = create_body["id"]
            .as_str()
            .expect("knowledge id")
            .to_string();

        // Update: remove one source, keep the other
        let update_req = test::TestRequest::put()
            .uri(&format!("/knowledges/{knowledge_id}"))
            .set_json(json!({
                "name": "기술 문서",
                "description": "API 관련 자료",
                "source_ids": [src1_id]
            }))
            .to_request();
        let update_body: Value = test::call_and_read_body_json(&app, update_req).await;
        let updated_ids = update_body["source_ids"]
            .as_array()
            .expect("source_ids should be array");
        assert_eq!(updated_ids.len(), 1);
        assert_eq!(updated_ids[0], Value::String(src1_id.clone()));
    }

    #[actix_web::test]
    async fn delete_source_cascades_to_knowledge_sources() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let state = test_state_with_upload_dir(&temp_dir).await;
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        // Upload a source
        let src = upload_source(&app, "cascade-test.pdf", b"cascade").await;
        let src_id = src["id"].as_str().expect("source id").to_string();

        // Create knowledge referencing the source
        let create_req = test::TestRequest::post()
            .uri("/knowledges")
            .set_json(json!({
                "name": "Cascade Test",
                "description": "test",
                "source_ids": [src_id]
            }))
            .to_request();
        let create_body: Value = test::call_and_read_body_json(&app, create_req).await;
        let knowledge_id = create_body["id"]
            .as_str()
            .expect("knowledge id")
            .to_string();

        // Verify source_ids contains the source
        assert_eq!(create_body["source_ids"], json!([src_id]));

        // Delete the source
        let delete_req = test::TestRequest::delete()
            .uri(&format!("/sources/{src_id}"))
            .to_request();
        let delete_resp = test::call_service(&app, delete_req).await;
        assert_eq!(delete_resp.status(), StatusCode::NO_CONTENT);

        // Knowledge should still exist, but source_ids should be empty (cascaded)
        let get_knowledge = test::TestRequest::get()
            .uri(&format!("/knowledges/{knowledge_id}"))
            .to_request();
        let knowledge_body: Value = test::call_and_read_body_json(&app, get_knowledge).await;
        assert_eq!(knowledge_body["source_ids"], json!([]));
    }

    #[actix_web::test]
    async fn knowledge_not_found_returns_404() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let database_url = test_database_url(&temp_dir);
        let state = web::Data::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        let app = test::init_service(App::new().app_data(state).configure(configure)).await;

        let fake_id = Uuid::new_v4();

        let get_req = test::TestRequest::get()
            .uri(&format!("/knowledges/{fake_id}"))
            .to_request();
        assert_eq!(
            test::call_service(&app, get_req).await.status(),
            StatusCode::NOT_FOUND
        );

        let update_req = test::TestRequest::put()
            .uri(&format!("/knowledges/{fake_id}"))
            .set_json(json!({"name": "x", "description": "y", "source_ids": []}))
            .to_request();
        assert_eq!(
            test::call_service(&app, update_req).await.status(),
            StatusCode::NOT_FOUND
        );

        let delete_req = test::TestRequest::delete()
            .uri(&format!("/knowledges/{fake_id}"))
            .to_request();
        assert_eq!(
            test::call_service(&app, delete_req).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
