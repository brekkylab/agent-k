use std::convert::Infallible;
use std::sync::Arc;

use aide::NoApi;
use aide::axum::ApiRouter;
use aide::axum::routing::{get, post};
use ailoy::{AgentProvider, LangModelAPISchema, LangModelProvider};
use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Response, Sse};
use futures_util::StreamExt;
use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

use crate::models::{
    AddSessionMessageRequest, Agent, AgentResponse, CreateAgentRequest,
    CreateProviderProfileRequest, CreateSessionRequest, CreateSpeedwagonRequest, ErrorResponse,
    ListSessionsQuery, ProviderProfile, ProviderProfileResponse, SessionDetailResponse,
    SessionResponse, SourceResponse, SourceType, SpeedwagonIndexStatus, SpeedwagonResponse,
    UpdateProviderProfileRequest, UpdateSessionRequest, UpdateSpeedwagonRequest,
};
use crate::repository::RepositoryError;
use crate::services::agent::{self as agent_service};
use crate::services::session::{self as session_service, SessionError, SseEvent};
use crate::services::speedwagon::{self as speedwagon_service, SpeedwagonError};
use crate::state::AppState;

type AppState_ = Arc<AppState>;
type ApiErr = (StatusCode, Json<AppError>);
type ApiResult<T> = Result<T, ApiErr>;

#[derive(Debug, JsonSchema, Serialize)]
struct AppError {
    error: String,
}

impl AppError {
    fn not_found(msg: impl Into<String>) -> ApiErr {
        (StatusCode::NOT_FOUND, Json(Self { error: msg.into() }))
    }
    fn conflict(msg: impl Into<String>) -> ApiErr {
        (StatusCode::CONFLICT, Json(Self { error: msg.into() }))
    }
    fn bad_request(msg: impl Into<String>) -> ApiErr {
        (StatusCode::BAD_REQUEST, Json(Self { error: msg.into() }))
    }
    fn internal(msg: impl Into<String>) -> ApiErr {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(Self { error: msg.into() }),
        )
    }
}

/// Return a copy of the provider with the Gemini URL normalized to base format (`/models/`).
///
/// ailoy constructs Gemini API URLs as `format!("{}{}:generateContent", url, model)`,
/// so the URL must end with `/models/` (base URL without model name or method).
/// Non-Gemini providers pass through unchanged.
fn normalize_provider(provider: AgentProvider) -> AgentProvider {
    let lm = match provider.lm {
        LangModelProvider::API {
            schema,
            url,
            api_key,
        } => {
            let url = if matches!(schema, LangModelAPISchema::Gemini) {
                let s = url.to_string();
                if let Some(idx) = s.find("/models/") {
                    let base = &s[..idx + 8];
                    base.parse().unwrap_or(url)
                } else {
                    url
                }
            } else {
                url
            };
            LangModelProvider::API {
                schema,
                url,
                api_key,
            }
        }
    };
    AgentProvider {
        lm,
        tools: provider.tools,
    }
}

fn repo_err(error: RepositoryError) -> ApiErr {
    if let RepositoryError::Database(sqlx::Error::Database(db_error)) = &error {
        let msg = db_error.message();
        if msg.contains("UNIQUE constraint failed: provider_profiles.name") {
            return AppError::conflict("provider profile name already exists");
        }
    }
    tracing::error!("repository error: {error}");
    AppError::internal("internal server error")
}

fn session_err(e: SessionError) -> ApiErr {
    let status = match &e {
        SessionError::NotFound
        | SessionError::AgentNotFound
        | SessionError::ProviderProfileNotFound => StatusCode::NOT_FOUND,
        SessionError::NoDefaultProviderProfile | SessionError::EmptyContent => {
            StatusCode::BAD_REQUEST
        }
        SessionError::Runtime(_) => StatusCode::BAD_GATEWAY,
        SessionError::Repository(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let msg = if matches!(e, SessionError::Repository(_)) {
        "internal server error".to_string()
    } else {
        e.to_string()
    };
    (status, Json(AppError { error: msg }))
}

fn speedwagon_err(e: SpeedwagonError) -> ApiErr {
    let status = match &e {
        SpeedwagonError::NotFound => StatusCode::NOT_FOUND,
        SpeedwagonError::AlreadyIndexing => StatusCode::CONFLICT,
        SpeedwagonError::NoSources => StatusCode::UNPROCESSABLE_ENTITY,
        SpeedwagonError::EmptyName | SpeedwagonError::InconsistentOverride => {
            StatusCode::BAD_REQUEST
        }
        SpeedwagonError::Repository(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let msg = if matches!(e, SpeedwagonError::Repository(_)) {
        "internal server error".to_string()
    } else {
        e.to_string()
    };
    (status, Json(AppError { error: msg }))
}

#[derive(Serialize, JsonSchema)]
struct HealthResponse {
    status: &'static str,
}

pub fn router(state: AppState_) -> ApiRouter {
    ApiRouter::new()
        .api_route("/health", get(health))
        .api_route("/agents", get(list_agents).post(create_agent))
        .api_route(
            "/agents/{id}",
            get(get_agent).delete(delete_agent),
        )
        .api_route(
            "/provider-profiles",
            get(list_provider_profiles).post(create_provider_profile),
        )
        .api_route(
            "/provider-profiles/{id}",
            get(get_provider_profile)
                .put(update_provider_profile)
                .delete(delete_provider_profile),
        )
        .api_route("/sessions", get(list_sessions).post(create_session))
        .api_route(
            "/sessions/{id}",
            get(get_session).put(update_session).delete(delete_session),
        )
        .api_route(
            "/sessions/{id}/messages/stream",
            post(add_message_streaming),
        )
        .api_route("/sessions/{id}/tool-calls", get(get_session_tool_calls))
        .api_route("/sources", get(list_sources).post(upload_source))
        .api_route("/sources/{id}", get(get_source).delete(delete_source))
        .api_route(
            "/speedwagons",
            get(list_speedwagons).post(create_speedwagon),
        )
        .api_route(
            "/speedwagons/{id}",
            get(get_speedwagon)
                .put(update_speedwagon)
                .delete(delete_speedwagon),
        )
        .api_route("/speedwagons/{id}/index", post(index_speedwagon))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn create_agent(
    State(state): State<AppState_>,
    Json(payload): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<AgentResponse>), (StatusCode, Json<AppError>)> {
    let CreateAgentRequest { spec } = payload;
    let (agent, created) = agent_service::find_or_create_agent(&state, spec)
        .await
        .map_err(repo_err)?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(to_agent_response(&agent))))
}

async fn list_agents(
    State(state): State<AppState_>,
) -> Result<Json<Vec<AgentResponse>>, (StatusCode, Json<AppError>)> {
    let agents = state.repository.list_agents().await.map_err(repo_err)?;
    Ok(Json(agents.iter().map(to_agent_response).collect()))
}

async fn get_agent(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<Json<AgentResponse>, (StatusCode, Json<AppError>)> {
    match state.repository.get_agent(id).await.map_err(repo_err)? {
        Some(agent) => Ok(Json(to_agent_response(&agent))),
        None => Err(AppError::not_found("agent not found")),
    }
}

async fn delete_agent(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
    if state
        .repository
        .has_sessions_for_agent(id)
        .await
        .map_err(repo_err)?
    {
        return Err(AppError::conflict(
            "cannot delete agent with existing sessions",
        ));
    }
    match state.repository.delete_agent(id).await.map_err(repo_err)? {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(AppError::not_found("agent not found")),
    }
}

async fn create_provider_profile(
    State(state): State<AppState_>,
    Json(payload): Json<CreateProviderProfileRequest>,
) -> Result<(StatusCode, Json<ProviderProfileResponse>), (StatusCode, Json<AppError>)> {
    let CreateProviderProfileRequest {
        name,
        provider,
        is_default,
    } = payload;

    if name.trim().is_empty() {
        return Err(AppError::bad_request("provider profile name is empty"));
    }

    let profile = state
        .repository
        .create_provider_profile(name, normalize_provider(provider), is_default)
        .await
        .map_err(repo_err)?;
    Ok((
        StatusCode::CREATED,
        Json(to_provider_profile_response(&profile)),
    ))
}

async fn list_provider_profiles(
    State(state): State<AppState_>,
) -> Result<Json<Vec<ProviderProfileResponse>>, (StatusCode, Json<AppError>)> {
    let profiles = state
        .repository
        .list_provider_profiles()
        .await
        .map_err(repo_err)?;
    Ok(Json(
        profiles.iter().map(to_provider_profile_response).collect(),
    ))
}

async fn get_provider_profile(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProviderProfileResponse>, (StatusCode, Json<AppError>)> {
    match state
        .repository
        .get_provider_profile(id)
        .await
        .map_err(repo_err)?
    {
        Some(profile) => Ok(Json(to_provider_profile_response(&profile))),
        None => Err(AppError::not_found("provider profile not found")),
    }
}

async fn update_provider_profile(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProviderProfileRequest>,
) -> Result<Json<ProviderProfileResponse>, (StatusCode, Json<AppError>)> {
    let UpdateProviderProfileRequest {
        name,
        provider,
        is_default,
    } = payload;

    if name.trim().is_empty() {
        return Err(AppError::bad_request("provider profile name is empty"));
    }

    match state
        .repository
        .update_provider_profile(id, name, normalize_provider(provider), is_default)
        .await
        .map_err(repo_err)?
    {
        Some(profile) => {
            state.invalidate_runtimes_by_provider_profile_id(id);
            Ok(Json(to_provider_profile_response(&profile)))
        }
        None => Err(AppError::not_found("provider profile not found")),
    }
}

async fn delete_provider_profile(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
    if state
        .repository
        .has_sessions_for_provider_profile(id)
        .await
        .map_err(repo_err)?
    {
        return Err(AppError::conflict(
            "cannot delete provider profile with existing sessions",
        ));
    }
    match state
        .repository
        .delete_provider_profile(id)
        .await
        .map_err(repo_err)?
    {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(AppError::not_found("provider profile not found")),
    }
}

async fn create_session(
    State(state): State<AppState_>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), (StatusCode, Json<AppError>)> {
    let session = session_service::create_session(&state, payload)
        .await
        .map_err(session_err)?;
    Ok((StatusCode::CREATED, Json(SessionResponse::from(&session))))
}

async fn list_sessions(
    State(state): State<AppState_>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionResponse>>, (StatusCode, Json<AppError>)> {
    let ListSessionsQuery {
        agent_id,
        include_messages,
    } = query;

    let sessions = state
        .repository
        .list_sessions(agent_id, include_messages.unwrap_or(false))
        .await
        .map_err(repo_err)?;
    Ok(Json(sessions.iter().map(SessionResponse::from).collect()))
}

async fn get_session(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SessionDetailResponse>> {
    match session_service::get_session_detail(&state, id)
        .await
        .map_err(session_err)?
    {
        Some(detail) => Ok(Json(detail)),
        None => Err(AppError::not_found("session not found")),
    }
}

async fn update_session(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateSessionRequest>,
) -> Result<Json<SessionResponse>, (StatusCode, Json<AppError>)> {
    let session = session_service::update_session(&state, id, payload)
        .await
        .map_err(session_err)?;
    Ok(Json(SessionResponse::from(&session)))
}

async fn delete_session(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
    session_service::delete_session(&state, id)
        .await
        .map_err(session_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_message_streaming(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
    NoApi(Json(payload)): NoApi<Json<AddSessionMessageRequest>>,
) -> NoApi<Response> {
    let AddSessionMessageRequest { content } = payload;

    let event_stream = match session_service::send_message_streaming(&state, id, content).await {
        Ok(stream) => stream,
        Err(e) => return NoApi(e.into_response()),
    };

    let sse_stream = event_stream.map(|event_result| {
        let event = match event_result {
            Ok(event) => event,
            Err(e) => SseEvent::Error {
                message: e.to_string(),
            },
        };
        let event_type = match &event {
            SseEvent::Thinking { .. } => "thinking",
            SseEvent::ToolCall { .. } => "tool_call",
            SseEvent::ToolResult { .. } => "tool_result",
            SseEvent::Message { .. } => "message",
            SseEvent::Done { .. } => "done",
            SseEvent::Error { .. } => "error",
        };
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().event(event_type).data(data))
    });

    NoApi(
        Sse::new(sse_stream)
            .keep_alive(KeepAlive::default())
            .into_response(),
    )
}

async fn get_session_tool_calls(
    State(state): State<AppState_>,
    Path(session_id): Path<Uuid>,
) -> Result<Response, (StatusCode, Json<AppError>)> {
    let tool_calls = state
        .repository
        .get_tool_calls_for_session(session_id)
        .await
        .map_err(repo_err)?;
    Ok(Json(tool_calls).into_response())
}

// ===================== Source Handlers =====================

async fn upload_source(
    State(state): State<AppState_>,
    NoApi(mut multipart): NoApi<Multipart>,
) -> NoApi<Response> {
    let mut file_name = String::new();
    let mut file_bytes: Vec<u8> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        if let Some(name) = field.file_name() {
            file_name = name.to_string();
        }
        match field.bytes().await {
            Ok(bytes) => file_bytes.extend_from_slice(&bytes),
            Err(e) => {
                tracing::error!("failed to read multipart field: {e}");
                return NoApi(json_error(StatusCode::BAD_REQUEST, "failed to read file"));
            }
        }
        // Only handle the first file
        break;
    }

    if file_name.is_empty() || file_bytes.is_empty() {
        return NoApi(json_error(StatusCode::BAD_REQUEST, "no file in request"));
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
        tracing::error!("failed to create upload dir: {error}");
        return NoApi(json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to create upload directory",
        ));
    }

    let file_path = upload_dir.join(&stored_name);
    if let Err(error) = tokio::fs::write(&file_path, &file_bytes).await {
        tracing::error!("failed to write file: {error}");
        return NoApi(json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to write file",
        ));
    }

    let file_path_str = file_path.to_string_lossy().to_string();

    match state
        .repository
        .create_source(file_name, SourceType::LocalFile, Some(file_path_str), size)
        .await
    {
        Ok(source) => {
            NoApi((StatusCode::CREATED, Json(SourceResponse::from(&source))).into_response())
        }
        Err(error) => NoApi(repository_error_response(error)),
    }
}

async fn list_sources(
    State(state): State<AppState_>,
) -> Result<Json<Vec<SourceResponse>>, (StatusCode, Json<AppError>)> {
    let sources = state.repository.list_sources().await.map_err(repo_err)?;
    Ok(Json(sources.iter().map(SourceResponse::from).collect()))
}

async fn get_source(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<Json<SourceResponse>, (StatusCode, Json<AppError>)> {
    match state.repository.get_source(id).await.map_err(repo_err)? {
        Some(source) => Ok(Json(SourceResponse::from(&source))),
        None => Err(AppError::not_found("source not found")),
    }
}

async fn delete_source(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
    // Get file_path before deleting from DB
    let file_path = match state.repository.get_source(id).await.map_err(repo_err)? {
        Some(source) => source.file_path,
        None => return Err(AppError::not_found("source not found")),
    };

    match state.repository.delete_source(id).await.map_err(repo_err)? {
        true => {
            // Delete file from disk
            if let Some(path) = &file_path {
                if let Err(error) = tokio::fs::remove_file(path).await {
                    tracing::warn!("failed to delete file {path}: {error}");
                }
            }

            // Reset index_status for any speedwagons that referenced this source
            if let Ok(speedwagons) = state.repository.list_speedwagons().await {
                for sw in speedwagons {
                    if sw.source_ids.contains(&id)
                        && sw.index_status != SpeedwagonIndexStatus::NotIndexed
                    {
                        let _ = state
                            .repository
                            .update_speedwagon_index_status(
                                sw.id,
                                SpeedwagonIndexStatus::NotIndexed,
                                None,
                                None,
                                None,
                                None,
                                None,
                            )
                            .await;
                    }
                }
            }

            Ok(StatusCode::NO_CONTENT)
        }
        false => Err(AppError::not_found("source not found")),
    }
}

// ===================== Speedwagon Handlers =====================

async fn create_speedwagon(
    State(state): State<AppState_>,
    Json(payload): Json<CreateSpeedwagonRequest>,
) -> Result<(StatusCode, Json<SpeedwagonResponse>), (StatusCode, Json<AppError>)> {
    let sw = speedwagon_service::create_speedwagon(&state, payload)
        .await
        .map_err(speedwagon_err)?;
    Ok((StatusCode::CREATED, Json(SpeedwagonResponse::from(&sw))))
}

async fn list_speedwagons(
    State(state): State<AppState_>,
) -> Result<Json<Vec<SpeedwagonResponse>>, (StatusCode, Json<AppError>)> {
    let list = state
        .repository
        .list_speedwagons()
        .await
        .map_err(repo_err)?;
    Ok(Json(list.iter().map(SpeedwagonResponse::from).collect()))
}

async fn get_speedwagon(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<Json<SpeedwagonResponse>, (StatusCode, Json<AppError>)> {
    match state
        .repository
        .get_speedwagon(id)
        .await
        .map_err(repo_err)?
    {
        Some(sw) => Ok(Json(SpeedwagonResponse::from(&sw))),
        None => Err(AppError::not_found("speedwagon not found")),
    }
}

async fn update_speedwagon(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateSpeedwagonRequest>,
) -> Result<Json<SpeedwagonResponse>, (StatusCode, Json<AppError>)> {
    let sw = speedwagon_service::update_speedwagon(&state, id, payload)
        .await
        .map_err(speedwagon_err)?;
    Ok(Json(SpeedwagonResponse::from(&sw)))
}

async fn delete_speedwagon(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
    speedwagon_service::delete_speedwagon(&state, id)
        .await
        .map_err(speedwagon_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn index_speedwagon(
    State(state): State<AppState_>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<AppError>)> {
    let sw = speedwagon_service::start_indexing(&state, id)
        .await
        .map_err(speedwagon_err)?;

    let repository = Arc::clone(&state.repository);
    let speedwagon_data_dir = state.speedwagon_data_dir.clone();
    let state_clone = Arc::clone(&state);

    tokio::task::spawn(async move {
        let _ = crate::services::indexing::start_indexing(
            repository,
            state_clone,
            speedwagon_data_dir,
            sw,
        )
        .await;
    });

    Ok(StatusCode::ACCEPTED)
}

fn to_agent_response(agent: &Agent) -> AgentResponse {
    AgentResponse {
        id: agent.id,
        spec: agent.spec.clone(),
        created_at: agent.created_at,
        updated_at: agent.updated_at,
    }
}

fn to_provider_profile_response(profile: &ProviderProfile) -> ProviderProfileResponse {
    let mut provider: AgentProvider = profile.provider.clone();
    let LangModelProvider::API { api_key, .. } = &mut provider.lm;
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

fn repository_error_response(error: RepositoryError) -> Response {
    if let RepositoryError::Database(sqlx::Error::Database(db_error)) = &error {
        let msg = db_error.message();
        if msg.contains("UNIQUE constraint failed: provider_profiles.name") {
            return json_error(StatusCode::CONFLICT, "provider profile name already exists");
        }
    }

    tracing::error!("repository error: {error}");
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}

fn json_error(status: StatusCode, error: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: error.into(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tower::ServiceExt;
    use uuid::Uuid;

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

    async fn test_app(temp_dir: &TempDir) -> axum::Router {
        let database_url = test_database_url(temp_dir);
        let state = Arc::new(
            AppState::new_without_bootstrap(&database_url)
                .await
                .expect("state should be created"),
        );
        super::router(state).into()
    }

    async fn test_app_with_upload_dir(temp_dir: &TempDir) -> axum::Router {
        let database_url = test_database_url(temp_dir);
        let upload_dir = temp_dir.path().join("uploads");
        let state = Arc::new(
            AppState::new_without_bootstrap_with_upload_dir(&database_url, upload_dir)
                .await
                .expect("state should be created"),
        );
        super::router(state).into()
    }

    async fn post_json(app: &axum::Router, uri: &str, body: Value) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn get_req(app: &axum::Router, uri: &str) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn put_json(app: &axum::Router, uri: &str, body: Value) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn delete_req(app: &axum::Router, uri: &str) -> Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn response_json(resp: Response) -> Value {
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn create_agent(app: &axum::Router) -> Uuid {
        let resp = post_json(
            app,
            "/agents",
            json!({
                "spec": {
                    "lm": "gpt-4.1",
                    "instruction": null,
                    "tools": []
                }
            }),
        )
        .await;
        let body = response_json(resp).await;
        Uuid::parse_str(body["id"].as_str().expect("agent id must exist"))
            .expect("agent id must be uuid")
    }

    async fn create_provider_profile(
        app: &axum::Router,
        name: &str,
        schema: &str,
        is_default: bool,
    ) -> Uuid {
        create_provider_profile_with_url(app, name, schema, "https://example.com/v1", is_default)
            .await
    }

    async fn create_provider_profile_with_url(
        app: &axum::Router,
        name: &str,
        schema: &str,
        url: &str,
        is_default: bool,
    ) -> Uuid {
        let resp = post_json(
            app,
            "/provider-profiles",
            json!({
                "name": name,
                "provider": provider_payload(schema, url, "secret-key"),
                "is_default": is_default
            }),
        )
        .await;
        let body = response_json(resp).await;
        Uuid::parse_str(body["id"].as_str().expect("provider profile id must exist"))
            .expect("provider profile id must be uuid")
    }

    async fn start_mock_chat_completion_server() -> (String, Arc<Mutex<Vec<usize>>>) {
        let request_message_counts = Arc::new(Mutex::new(Vec::new()));
        let counts_state = Arc::clone(&request_message_counts);

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind should work");
        listener
            .set_nonblocking(true)
            .expect("set_nonblocking should work");
        let addr = listener
            .local_addr()
            .expect("local addr should be available");

        let mock_app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let counts_state = Arc::clone(&counts_state);
                async move {
                    let message_count = body
                        .get("messages")
                        .and_then(Value::as_array)
                        .map(|messages| messages.len())
                        .unwrap_or(0);

                    counts_state
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

                    axum::Json(json!({
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
                }
            }),
        );

        tokio::spawn(async move {
            axum::serve(
                tokio::net::TcpListener::from_std(listener).unwrap(),
                mock_app,
            )
            .await
            .unwrap();
        });

        (
            format!("http://{addr}/v1/chat/completions"),
            request_message_counts,
        )
    }

    /// SSE response body to (event_type, json_data) pairs.
    fn parse_sse_events(body: &[u8]) -> Vec<(String, Value)> {
        let text = std::str::from_utf8(body).unwrap_or("");
        let mut events = Vec::new();
        let mut current_event: Option<String> = None;
        let mut current_data: Option<String> = None;
        for line in text.lines() {
            if let Some(ev) = line.strip_prefix("event: ") {
                current_event = Some(ev.to_string());
            } else if let Some(dat) = line.strip_prefix("data: ") {
                current_data = Some(dat.to_string());
            } else if line.is_empty() {
                if let (Some(ev), Some(dat)) = (current_event.take(), current_data.take()) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&dat) {
                        events.push((ev, parsed));
                    }
                }
            }
        }
        events
    }

    /// Send a user message over SSE stream and return (StatusCode, parsed events).
    async fn stream_user_message(
        app: &axum::Router,
        session_id: &str,
        content: &str,
    ) -> (StatusCode, Vec<(String, Value)>) {
        let resp = post_json(
            app,
            &format!("/sessions/{session_id}/messages/stream"),
            json!({ "content": content }),
        )
        .await;
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, parse_sse_events(&body))
    }

    #[tokio::test]
    async fn create_agent_rejects_provider_field() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let resp = post_json(
            &app,
            "/agents",
            json!({
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
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn provider_profile_api_hides_api_key() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let create_resp = post_json(
            &app,
            "/provider-profiles",
            json!({
                "name": "openai-default",
                "provider": provider_payload(
                    "chat_completion",
                    "https://api.openai.com/v1/chat/completions",
                    "very-secret"
                ),
                "is_default": true
            }),
        )
        .await;
        let create_body = response_json(create_resp).await;
        assert!(create_body["provider"]["lm"]["api_key"].is_null());

        let profile_id = create_body["id"].as_str().expect("profile id must exist");

        let get_resp = get_req(&app, &format!("/provider-profiles/{profile_id}")).await;
        let get_body = response_json(get_resp).await;
        assert!(get_body["provider"]["lm"]["api_key"].is_null());

        let list_resp = get_req(&app, "/provider-profiles").await;
        let list_body = response_json(list_resp).await;
        assert!(
            list_body.as_array().expect("list should be array")[0]["provider"]["lm"]["api_key"]
                .is_null()
        );
    }

    #[tokio::test]
    async fn create_session_with_explicit_profile_or_missing_profile() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let session_resp = post_json(
            &app,
            "/sessions",
            json!({
                "agent_id": agent_id,
                "provider_profile_id": profile_id
            }),
        )
        .await;
        let session_body = response_json(session_resp).await;
        assert_eq!(
            session_body["provider_profile_id"],
            Value::String(profile_id.to_string())
        );

        let missing_resp = post_json(
            &app,
            "/sessions",
            json!({
                "agent_id": agent_id,
                "provider_profile_id": Uuid::new_v4()
            }),
        )
        .await;
        assert_eq!(missing_resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn session_default_profile_selection_follows_priority() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;

        let _gemini_id = create_provider_profile(&app, "gemini-default", "gemini", true).await;
        let _anthropic_id =
            create_provider_profile(&app, "anthropic-default", "anthropic", true).await;
        let openai_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let session_resp = post_json(&app, "/sessions", json!({ "agent_id": agent_id })).await;
        let session_body = response_json(session_resp).await;
        assert_eq!(
            session_body["provider_profile_id"],
            Value::String(openai_id.to_string())
        );
    }

    #[tokio::test]
    async fn session_default_profile_tiebreak_uses_created_order() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let first = create_provider_profile(&app, "openai-first", "chat_completion", true).await;
        tokio::time::sleep(Duration::from_millis(2)).await;
        let _second = create_provider_profile(&app, "openai-second", "chat_completion", true).await;

        let session_resp = post_json(&app, "/sessions", json!({ "agent_id": agent_id })).await;
        let session_body = response_json(session_resp).await;
        assert_eq!(
            session_body["provider_profile_id"],
            Value::String(first.to_string())
        );
    }

    #[tokio::test]
    async fn create_session_without_default_profile_returns_bad_request() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let _profile_id =
            create_provider_profile(&app, "not-default", "chat_completion", false).await;

        let resp = post_json(&app, "/sessions", json!({ "agent_id": agent_id })).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_provider_profile_in_use_returns_conflict() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let _session_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({
                    "agent_id": agent_id,
                    "provider_profile_id": profile_id
                }),
            )
            .await,
        )
        .await;

        let delete_resp = delete_req(&app, &format!("/provider-profiles/{profile_id}")).await;
        assert_eq!(delete_resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn add_message_runtime_failure_returns_error_event_and_keeps_user_message() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let profile_id = create_provider_profile_with_url(
            &app,
            "openai-default",
            "chat_completion",
            "http://127.0.0.1:1/v1/chat/completions",
            true,
        )
        .await;

        let session_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({
                    "agent_id": agent_id,
                    "provider_profile_id": profile_id
                }),
            )
            .await,
        )
        .await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let (status, events) = stream_user_message(&app, &session_id, "hello").await;
        // Runtime creation may succeed but the LLM call fails inside the stream
        if status == StatusCode::OK {
            let has_error = events.iter().any(|(e, _)| e == "error");
            assert!(
                has_error,
                "expected error event in SSE stream on connection failure"
            );
        } else {
            assert_eq!(status, StatusCode::BAD_GATEWAY);
        }

        let updated_session =
            response_json(get_req(&app, &format!("/sessions/{session_id}")).await).await;
        let messages = updated_session["messages"]
            .as_array()
            .expect("messages should be an array");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], Value::String("user".to_string()));
        assert_eq!(messages[0]["content"], Value::String("hello".to_string()));
    }

    #[tokio::test]
    async fn add_message_user_inference_uses_runtime_history_between_turns() {
        let (mock_url, request_counts) = start_mock_chat_completion_server().await;
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let profile_id = create_provider_profile_with_url(
            &app,
            "openai-default",
            "chat_completion",
            &mock_url,
            true,
        )
        .await;

        let session_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({
                    "agent_id": agent_id,
                    "provider_profile_id": profile_id
                }),
            )
            .await,
        )
        .await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let (status1, first_events) = stream_user_message(&app, &session_id, "turn-1").await;
        assert_eq!(status1, StatusCode::OK);
        let first_done = first_events
            .iter()
            .find(|(e, _)| e == "done")
            .expect("done event must be present for turn-1");
        assert_eq!(
            first_done.1["assistant_message"]["content"],
            Value::String("assistant:turn-1".to_string())
        );

        let (status2, second_events) = stream_user_message(&app, &session_id, "turn-2").await;
        assert_eq!(status2, StatusCode::OK);
        let second_done = second_events
            .iter()
            .find(|(e, _)| e == "done")
            .expect("done event must be present for turn-2");
        assert_eq!(
            second_done.1["assistant_message"]["content"],
            Value::String("assistant:turn-2".to_string())
        );

        let counts = request_counts
            .lock()
            .expect("request counts lock should be available")
            .clone();
        // SSE path includes system prompt + DB-restored history in each LLM call:
        // turn-1: system(1) + restored-user(1) + streaming-user(1) = 3
        // turn-2: system(1) + history(2) + restored-user(1) + streaming-user(1) = 5
        assert_eq!(counts, vec![3, 5]);
    }

    #[tokio::test]
    async fn update_provider_profile_resets_session_runtime_cache() {
        let (mock_url, request_counts) = start_mock_chat_completion_server().await;
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let profile_id = create_provider_profile_with_url(
            &app,
            "openai-default",
            "chat_completion",
            &mock_url,
            true,
        )
        .await;

        let session_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({
                    "agent_id": agent_id,
                    "provider_profile_id": profile_id
                }),
            )
            .await,
        )
        .await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        let (status, _) = stream_user_message(&app, &session_id, "before-provider-update").await;
        assert_eq!(status, StatusCode::OK);

        let update_resp = put_json(
            &app,
            &format!("/provider-profiles/{profile_id}"),
            json!({
                "name": "openai-default",
                "provider": provider_payload("chat_completion", &mock_url, "another-secret"),
                "is_default": true
            }),
        )
        .await;
        assert_eq!(update_resp.status(), StatusCode::OK);

        let (status, _) = stream_user_message(&app, &session_id, "after-provider-update").await;
        assert_eq!(status, StatusCode::OK);

        let counts = request_counts
            .lock()
            .expect("request counts lock should be available")
            .clone();
        // After provider profile update, runtime cache is invalidated and recreated with DB history restore.
        // turn-1: system(1) + restored-user(1) + streaming-user(1) = 3
        // turn-2 (after reset): system(1) + restored-history(2) + restored-user(1) + streaming-user(1) = 5
        assert_eq!(counts, vec![3, 5]);
    }

    #[tokio::test]
    async fn delete_session_removes_session_and_close_route_is_absent() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let session_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({
                    "agent_id": agent_id,
                    "provider_profile_id": profile_id
                }),
            )
            .await,
        )
        .await;
        let session_id = session_body["id"]
            .as_str()
            .expect("session id must exist")
            .to_string();

        // /close endpoint does not exist on this API
        let close_resp = post_json(&app, &format!("/sessions/{session_id}/close"), json!({})).await;
        assert_eq!(close_resp.status(), StatusCode::NOT_FOUND);

        let delete_resp = delete_req(&app, &format!("/sessions/{session_id}")).await;
        assert_eq!(delete_resp.status(), StatusCode::NO_CONTENT);

        let get_resp = get_req(&app, &format!("/sessions/{session_id}")).await;
        assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);

        let delete_profile_resp =
            delete_req(&app, &format!("/provider-profiles/{profile_id}")).await;
        assert_eq!(delete_profile_resp.status(), StatusCode::NO_CONTENT);
    }

    // ===================== Source Tests =====================

    fn multipart_file_payload(filename: &str, content: &[u8]) -> (String, Vec<u8>) {
        let boundary = "----TestBoundary7MA4YWxkTrZu0gW";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (boundary.to_string(), body)
    }

    async fn upload_source_req(app: &axum::Router, filename: &str, content: &[u8]) -> Value {
        let (boundary, body) = multipart_file_payload(filename, content);
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sources")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        response_json(resp).await
    }

    #[tokio::test]
    async fn source_upload_and_list() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app_with_upload_dir(&temp_dir).await;

        let source = upload_source_req(&app, "test.txt", b"hello world").await;
        assert!(source["id"].is_string());
        assert_eq!(source["name"], Value::String("test.txt".to_string()));
        assert_eq!(source["size"], Value::Number(11.into()));

        let list_resp = get_req(&app, "/sources").await;
        let list_body = response_json(list_resp).await;
        assert_eq!(list_body.as_array().expect("list should be array").len(), 1);
    }

    // ===================== find-or-create Agent Tests =====================

    #[tokio::test]
    async fn find_or_create_same_spec_returns_same_id() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let spec = json!({ "spec": { "lm": "gpt-4.1", "instruction": null, "tools": [] } });

        let resp1 = post_json(&app, "/agents", spec.clone()).await;
        assert_eq!(resp1.status(), StatusCode::CREATED);
        let body1 = response_json(resp1).await;

        let resp2 = post_json(&app, "/agents", spec.clone()).await;
        assert_eq!(resp2.status(), StatusCode::OK);
        let body2 = response_json(resp2).await;

        assert_eq!(
            body1["id"], body2["id"],
            "same spec must return same agent id"
        );

        // Verify only one row in DB
        let list = response_json(get_req(&app, "/agents").await).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn find_or_create_tools_order_independent() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let resp1 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "gpt-4.1", "tools": ["b", "a"] } }),
        )
        .await;
        assert_eq!(resp1.status(), StatusCode::CREATED);
        let id1 = response_json(resp1).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        let resp2 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "gpt-4.1", "tools": ["a", "b"] } }),
        )
        .await;
        assert_eq!(resp2.status(), StatusCode::OK);
        let id2 = response_json(resp2).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        assert_eq!(id1, id2, "tools order must not affect agent identity");
    }

    #[tokio::test]
    async fn find_or_create_empty_instruction_variants_match() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let resp1 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "gpt-4.1", "instruction": null, "tools": [] } }),
        )
        .await;
        assert_eq!(resp1.status(), StatusCode::CREATED);
        let id1 = response_json(resp1).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        // Some("") should normalize to None
        let resp2 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "gpt-4.1", "instruction": "", "tools": [] } }),
        )
        .await;
        assert_eq!(resp2.status(), StatusCode::OK);
        let id2 = response_json(resp2).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        assert_eq!(
            id1, id2,
            "null and empty instruction must produce same agent"
        );
    }

    #[tokio::test]
    async fn find_or_create_different_lm_creates_new_agent() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let resp1 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "gpt-4.1", "tools": [] } }),
        )
        .await;
        assert_eq!(resp1.status(), StatusCode::CREATED);
        let id1 = response_json(resp1).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        let resp2 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "claude-opus-4-5", "tools": [] } }),
        )
        .await;
        assert_eq!(resp2.status(), StatusCode::CREATED);
        let id2 = response_json(resp2).await["id"]
            .as_str()
            .unwrap()
            .to_string();

        assert_ne!(id1, id2, "different lm must create a new agent");
    }

    // ===================== Session agent_id relink Tests =====================

    #[tokio::test]
    async fn update_session_agent_id_relinks_session() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id1 = create_agent(&app).await;
        // Create a second agent with a different spec
        let resp2 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "claude-opus-4-5", "tools": [] } }),
        )
        .await;
        let agent_id2 =
            Uuid::parse_str(response_json(resp2).await["id"].as_str().expect("agent id")).unwrap();
        assert_ne!(agent_id1, agent_id2);

        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let session_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({ "agent_id": agent_id1, "provider_profile_id": profile_id }),
            )
            .await,
        )
        .await;
        let session_id = Uuid::parse_str(session_body["id"].as_str().expect("session id")).unwrap();
        assert_eq!(
            session_body["agent_id"].as_str().unwrap(),
            agent_id1.to_string()
        );

        // Relink to agent2
        let update_resp = put_json(
            &app,
            &format!("/sessions/{session_id}"),
            json!({ "agent_id": agent_id2 }),
        )
        .await;
        assert_eq!(update_resp.status(), StatusCode::OK);
        let updated = response_json(update_resp).await;
        assert_eq!(updated["agent_id"].as_str().unwrap(), agent_id2.to_string());
    }

    // ===================== Cascade Delete Tests =====================

    #[tokio::test]
    async fn delete_session_cascade_deletes_orphaned_agent() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        // Create agent + profile + session
        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let session_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({ "agent_id": agent_id, "provider_profile_id": profile_id }),
            )
            .await,
        )
        .await;
        let session_id = Uuid::parse_str(session_body["id"].as_str().expect("session id")).unwrap();

        // Delete the only session → agent should be deleted too
        let del_resp = delete_req(&app, &format!("/sessions/{session_id}")).await;
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

        let agent_resp = get_req(&app, &format!("/agents/{agent_id}")).await;
        assert_eq!(agent_resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_session_keeps_agent_when_other_sessions_exist() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id = create_agent(&app).await;
        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        // Create two sessions for the same agent
        let session1_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({ "agent_id": agent_id, "provider_profile_id": profile_id }),
            )
            .await,
        )
        .await;
        let session1_id =
            Uuid::parse_str(session1_body["id"].as_str().expect("session id")).unwrap();

        let _session2_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({ "agent_id": agent_id, "provider_profile_id": profile_id }),
            )
            .await,
        )
        .await;

        // Delete session1 only → agent must still exist
        let del_resp = delete_req(&app, &format!("/sessions/{session1_id}")).await;
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

        let agent_resp = get_req(&app, &format!("/agents/{agent_id}")).await;
        assert_eq!(agent_resp.status(), StatusCode::OK);
    }

    // ===================== Update-Session Orphan Cleanup (CoW Symmetry) =====================

    /// When `PUT /sessions/{id}` swaps agent_id on the session's *only* session,
    /// the previous agent becomes an orphan and must be deleted — symmetric with
    /// `delete_session_cascade_deletes_orphaned_agent`.
    #[tokio::test]
    async fn update_session_agent_swap_cascade_deletes_orphaned_previous_agent() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id1 = create_agent(&app).await;
        let resp2 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "claude-opus-4-5", "tools": [] } }),
        )
        .await;
        let agent_id2 =
            Uuid::parse_str(response_json(resp2).await["id"].as_str().expect("agent id")).unwrap();
        assert_ne!(agent_id1, agent_id2);

        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        let session_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({ "agent_id": agent_id1, "provider_profile_id": profile_id }),
            )
            .await,
        )
        .await;
        let session_id = Uuid::parse_str(session_body["id"].as_str().expect("session id")).unwrap();

        // Swap to agent2 — this is the only session pointing at agent1, so agent1 is orphaned.
        let update_resp = put_json(
            &app,
            &format!("/sessions/{session_id}"),
            json!({ "agent_id": agent_id2 }),
        )
        .await;
        assert_eq!(update_resp.status(), StatusCode::OK);

        // agent1 was orphaned by the swap → must be deleted.
        let agent1_resp = get_req(&app, &format!("/agents/{agent_id1}")).await;
        assert_eq!(
            agent1_resp.status(),
            StatusCode::NOT_FOUND,
            "previous agent should have been cleaned up after pointer swap"
        );

        // agent2 still has a session → must remain.
        let agent2_resp = get_req(&app, &format!("/agents/{agent_id2}")).await;
        assert_eq!(agent2_resp.status(), StatusCode::OK);
    }

    /// When another session still references the previous agent, the swap must
    /// NOT delete that agent.
    #[tokio::test]
    async fn update_session_agent_swap_keeps_previous_agent_when_other_sessions_exist() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let app = test_app(&temp_dir).await;

        let agent_id1 = create_agent(&app).await;
        let resp2 = post_json(
            &app,
            "/agents",
            json!({ "spec": { "lm": "claude-opus-4-5", "tools": [] } }),
        )
        .await;
        let agent_id2 =
            Uuid::parse_str(response_json(resp2).await["id"].as_str().expect("agent id")).unwrap();

        let profile_id =
            create_provider_profile(&app, "openai-default", "chat_completion", true).await;

        // Two sessions both pointing at agent1.
        let session1_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({ "agent_id": agent_id1, "provider_profile_id": profile_id }),
            )
            .await,
        )
        .await;
        let session1_id =
            Uuid::parse_str(session1_body["id"].as_str().expect("session id")).unwrap();

        let _session2_body = response_json(
            post_json(
                &app,
                "/sessions",
                json!({ "agent_id": agent_id1, "provider_profile_id": profile_id }),
            )
            .await,
        )
        .await;

        // Swap session1 to agent2. agent1 is still referenced by session2.
        let update_resp = put_json(
            &app,
            &format!("/sessions/{session1_id}"),
            json!({ "agent_id": agent_id2 }),
        )
        .await;
        assert_eq!(update_resp.status(), StatusCode::OK);

        // agent1 must still exist — session2 depends on it.
        let agent1_resp = get_req(&app, &format!("/agents/{agent_id1}")).await;
        assert_eq!(agent1_resp.status(), StatusCode::OK);
    }
}
