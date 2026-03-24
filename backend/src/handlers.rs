use actix_web::http::StatusCode;
use actix_web::{HttpResponse, web};
use ailoy::{
    AgentProvider, LangModelAPISchema, LangModelProvider, Message as RuntimeMessage,
    Part as RuntimePart, Role as RuntimeRole,
};
use serde::Serialize;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use crate::agent::spec::{
    AgentProvider as ApiAgentProvider, LangModelProvider as ApiLangModelProvider,
};
use crate::models::{
    AddSessionMessageRequest, AddSessionMessageResponse, Agent, AgentResponse, CreateAgentRequest,
    CreateProviderProfileRequest, CreateSessionRequest, ErrorResponse, ListSessionsQuery,
    MessageRole, ProviderProfile, ProviderProfileResponse, Session, SessionMessage,
    UpdateAgentRequest, UpdateProviderProfileRequest, UpdateSessionRequest,
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
        add_message
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
        (name = "sessions", description = "Session endpoints")
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
        .service(web::resource("/sessions/{id}/messages").route(web::post().to(add_message)));
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
        .create_session(agent_id, resolved_provider_profile_id, title)
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
    let UpdateSessionRequest { title } = payload.into_inner();

    match state.repository.update_session_title(id, title).await {
        Ok(true) => match state.repository.get_session(id).await {
            Ok(Some(session)) => HttpResponse::Ok().json(session),
            Ok(None) => json_error(StatusCode::NOT_FOUND, "session not found"),
            Err(error) => repository_error_response(error),
        },
        Ok(false) => json_error(StatusCode::NOT_FOUND, "session not found"),
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

    let query = RuntimeMessage::new(RuntimeRole::User).with_contents([RuntimePart::text(content)]);

    let runtime_output = {
        let mut runtime = runtime.lock().await;
        runtime.run(query).await
    };

    let assistant_text = match runtime_output {
        Ok(message) => match extract_assistant_text(&message) {
            Some(text) => text,
            None => {
                return json_error(
                    StatusCode::BAD_GATEWAY,
                    "model response did not include text content",
                );
            }
        },
        Err(error) => {
            eprintln!("runtime execution error: {error}");
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

fn extract_assistant_text(message: &RuntimeMessage) -> Option<String> {
    let text = message
        .contents
        .iter()
        .filter_map(|part| part.as_text())
        .collect::<Vec<_>>()
        .join("");

    if text.is_empty() { None } else { Some(text) }
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
}
