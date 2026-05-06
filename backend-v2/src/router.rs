use std::convert::Infallible;
use std::sync::Arc;

use aide::axum::{
    ApiRouter,
    routing::{delete, post},
};
use ailoy::{
    agent::{Agent, AgentBuilder, AgentCard},
    message::{Message, MessageOutput, Part, Role},
    runenv::{Sandbox, SandboxConfig},
};
use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use chrono::Utc;
use futures_util::StreamExt;
use serde::Deserialize;
use speedwagon::{FileType, SpeedwagonSpec};
use uuid::Uuid;

use crate::{
    error::{ApiResult, AppError},
    model::{
        BatchIngestResponse, BatchPurgeResponse, BulkPurgeRequest, CreateSessionRequest,
        DocumentResponse, FailedItem, SendMessageRequest, SendMessageResponse, SessionResponse,
    },
    state::AppState,
};

const DEFAULT_MODEL: &str = "openai/gpt-5.4-mini";

fn sandbox_name_for(id: &Uuid) -> String {
    let s = id.simple().to_string();
    format!("session-{}", &s[..12])
}

pub fn get_router(state: Arc<AppState>) -> ApiRouter {
    ApiRouter::new()
        .api_route("/sessions", post(create_session))
        .api_route("/sessions/{id}", delete(delete_session))
        .api_route("/sessions/{id}/messages", post(send_message))
        .route(
            "/sessions/{id}/messages/stream",
            axum::routing::post(send_message_stream),
        )
        .route(
            "/sessions/{id}/messages",
            axum::routing::get(get_message_history).delete(clear_message_history),
        )
        .route(
            "/documents",
            axum::routing::get(list_documents)
                .post(ingest_document)
                .delete(purge_documents),
        )
        .route(
            "/documents/{id}",
            axum::routing::get(get_document).delete(purge_document),
        )
        .with_state(state)
}

async fn build_agent(sandbox: Sandbox) -> Result<Agent, String> {
    let sw_card = AgentCard {
        name: "speedwagon".into(),
        description: "Search the knowledge base for answers. \
            This tool has access to uploaded documents that may contain \
            information the model doesn't have. \
            Use it for any question that could be answered from the knowledge base."
            .into(),
        skills: vec![],
    };
    let sw_spec = SpeedwagonSpec::new().card(sw_card.clone()).into_spec();

    AgentBuilder::new(DEFAULT_MODEL)
        .instruction(concat!(
            "You are a versatile assistant with access to code execution tools ",
            "(bash, python), web search, and a knowledge base (speedwagon). ",
            "You MUST use the speedwagon tool to search the document corpus ",
            "before answering ANY factual question — even if you think you already know the answer. ",
            "The corpus contains authoritative information that may differ from your training data. ",
            "Use bash and python tools for computation, data analysis, and code execution tasks. ",
            "Only skip tools for greetings or casual conversation.",
        ))
        .tool("bash")
        .tool("python_repl")
        .tool("web_search")
        .runenv(sandbox)
        .subagent(sw_spec)
        .build()
        .await
        .map_err(|e| e.to_string())
}

// Alternative: main agent uses speedwagon tools directly (no subagent delegation).
// Materialize speedwagon ToolFactory entries for the main agent's spec so it can
// call search functions itself, instead of routing through a dedicated subagent.
//
// async fn build_agent(sandbox: Arc<Sandbox>, toolset: &ToolSet) -> Result<Agent, String> {
//     let (bash, python, web_search) = tokio::try_join!(
//         make_builtin_tool(&BuiltinToolProvider::Bash {}),
//         make_builtin_tool(&BuiltinToolProvider::PythonRepl {}),
//         make_builtin_tool(&BuiltinToolProvider::WebSearch {}),
//     )
//     .map_err(|e| e.to_string())?;

//     let model = build_lang_model(DEFAULT_MODEL)?;
//     let stub_spec = AgentSpec::new(DEFAULT_MODEL);

//     let mut builder = AgentBuilder::new(model)
//         .instruction(concat!(
//             "You are a versatile assistant with access to code execution tools ",
//             "(bash, python), web search, and a knowledge base. ",
//             "You MUST use the knowledge base search tools ",
//             "before answering ANY factual question. ",
//             "Use bash and python tools for computation and code execution tasks. ",
//             "Only skip tools for greetings or casual conversation.",
//         ))
//         .tool(bash)
//         .tool(python)
//         .tool(web_search)
//         .runenv(sandbox);

//     // Materialize each speedwagon ToolFactory into a concrete Tool.
//     // ToolFactory::make(spec) selects the right implementation (e.g. sandbox-aware).
//     for (_name, factory) in toolset.iter() {
//         builder = builder.tool(factory.make(&stub_spec));
//     }

//     builder.build().await.map_err(|e| e.to_string())
// }

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(_payload): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionResponse>)> {
    let id = Uuid::new_v4();
    let sandbox_name = sandbox_name_for(&id);

    let cfg = SandboxConfig {
        name: Some(sandbox_name.clone()),
        persist: true,
        ..Default::default()
    };
    let sandbox = Sandbox::new(cfg)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let agent = build_agent(sandbox)
        .await
        .map_err(|e| AppError::internal(e))?;

    let now = Utc::now();
    state
        .repository
        .create_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    state.insert_agent(id, agent);

    tracing::info!(%id, sandbox = %sandbox_name, "session created");

    Ok((
        StatusCode::CREATED,
        Json(SessionResponse {
            id,
            created_at: now,
            updated_at: now,
        }),
    ))
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }

    state
        .repository
        .delete_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    let agent_arc = state.remove_agent(&id);

    if let Some(arc) = agent_arc {
        drop(arc.lock().await);
        drop(arc);
    }

    let sandbox_name = sandbox_name_for(&id);
    if let Err(e) = ailoy::runenv::remove_persisted(&sandbox_name).await {
        tracing::warn!(%id, "failed to remove persisted sandbox: {e}");
    }

    tracing::info!(%id, "session deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// Resolve or lazy-create the in-memory agent for `id`.
///
/// On the first request after a server restart the agent is not in memory but
/// the session and its message history are in the DB. This function rebuilds
/// the agent and restores the history so the next turn starts with full context.
async fn resolve_agent(
    state: &Arc<AppState>,
    id: Uuid,
) -> ApiResult<Arc<tokio::sync::Mutex<Agent>>> {
    if let Some(arc) = state.get_agent(&id) {
        return Ok(arc);
    }

    let session_exists = state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_some();

    if !session_exists {
        return Err(AppError::not_found("session not found"));
    }

    let history = state
        .repository
        .get_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let sandbox_name = sandbox_name_for(&id);
    let cfg = SandboxConfig {
        name: Some(sandbox_name),
        persist: true,
        ..Default::default()
    };
    let sandbox = Sandbox::new(cfg)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let mut agent = build_agent(sandbox)
        .await
        .map_err(|e| AppError::internal(e))?;

    agent.state.history = history;
    tracing::info!(%id, "agent lazy-created with history restored");

    if let Some(existing) = state.get_agent(&id) {
        return Ok(existing);
    }
    state.insert_agent(id, agent);
    Ok(state.get_agent(&id).unwrap())
}

async fn get_message_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<Message>>> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }
    let messages = state
        .repository
        .get_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(messages))
}

async fn clear_message_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if state
        .repository
        .get_session(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_none()
    {
        return Err(AppError::not_found("session not found"));
    }
    state
        .repository
        .clear_messages(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    if let Some(arc) = state.get_agent(&id) {
        arc.lock().await.state.history.clear();
    }

    tracing::info!(%id, "message history cleared");
    Ok(StatusCode::NO_CONTENT)
}

async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<Json<SendMessageResponse>> {
    let agent_arc = resolve_agent(&state, id).await?;

    let prev_len = agent_arc.lock().await.get_history().len();

    let outputs = {
        let mut agent = agent_arc.lock().await;
        let msg = Message::new(Role::User).with_contents([Part::text(payload.content)]);
        let mut stream = agent.run(msg);
        let mut outputs: Vec<MessageOutput> = Vec::new();
        while let Some(item) = stream.next().await {
            outputs.push(item.map_err(|e| AppError::internal(e.to_string()))?);
        }
        outputs
    };

    let new_messages = {
        let agent = agent_arc.lock().await;
        agent.get_history()[prev_len..].to_vec()
    };
    state
        .repository
        .append_messages(id, &new_messages)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(outputs))
}

async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<SendMessageRequest>,
) -> ApiResult<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static>> {
    let agent_arc = resolve_agent(&state, id).await?;
    let repo = state.repository.clone();
    let prev_len = agent_arc.lock().await.get_history().len();
    let content = payload.content;

    let stream = async_stream::stream! {
        let mut agent = agent_arc.lock().await;
        let msg = Message::new(Role::User).with_contents([Part::text(content)]);
        let mut run = agent.run(msg);

        while let Some(item) = run.next().await {
            match item {
                Ok(output) => {
                    let json = serde_json::to_string(&output)
                        .unwrap_or_else(|e| format!("{{\"error\":\"{e}}}", e = e));
                    yield Ok::<Event, Infallible>(
                        Event::default().event("message").data(json),
                    );
                }
                Err(e) => {
                    yield Ok(Event::default().event("error").data(e.to_string()));
                    return;
                }
            }
        }
        drop(run);

        let new_msgs = agent.get_history()[prev_len..].to_vec();
        if let Err(e) = repo.append_messages(id, &new_msgs).await {
            tracing::error!(%id, "failed to persist messages: {e}");
        }

        yield Ok(Event::default().event("done").data("[DONE]"));
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ── Document endpoints ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ListDocumentsQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
}

async fn list_documents(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListDocumentsQuery>,
) -> ApiResult<Json<Vec<DocumentResponse>>> {
    let page = query.page.unwrap_or(0);
    let page_size = query.page_size.unwrap_or(50);

    let store = state.store.read().await;
    let docs = store
        .list(false, page, page_size)
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(docs.into_iter().map(DocumentResponse::from).collect()))
}

async fn get_document(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DocumentResponse>> {
    let store = state.store.read().await;
    match store.get(id) {
        Some(doc) => Ok(Json(DocumentResponse::from(doc))),
        None => Err(AppError::not_found("document not found")),
    }
}

fn parse_filetype(filename: &str) -> Result<FileType, String> {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" => Ok(FileType::PDF),
        "md" | "markdown" | "txt" => Ok(FileType::MD),
        _ => Err(format!(
            "unsupported file type '.{ext}' — supported: pdf, md, txt"
        )),
    }
}

async fn ingest_document(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<BatchIngestResponse>)> {
    let mut valid_items: Vec<(Vec<u8>, FileType)> = Vec::new();
    let mut filenames: Vec<String> = Vec::new();
    let mut failed: Vec<FailedItem> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or("upload").to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| AppError::internal(format!("failed to read upload: {e}")))?;

        match parse_filetype(&filename) {
            Ok(filetype) => {
                valid_items.push((bytes.to_vec(), filetype));
                filenames.push(filename);
            }
            Err(e) => {
                failed.push(FailedItem {
                    name: filename,
                    error: e,
                });
            }
        }
    }

    if valid_items.is_empty() && failed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(AppError::new("missing 'file' field in multipart body")),
        ));
    }

    let mut store = state.store.write().await;
    let result = store
        .ingest_many(valid_items)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let docs = store
        .get_many(&result.succeeded)
        .map_err(|e| AppError::internal(e.to_string()))?;
    drop(store);

    for f in result.failed {
        let name = filenames
            .get(f.index)
            .cloned()
            .unwrap_or_else(|| format!("file[{}]", f.index));
        failed.push(FailedItem {
            name,
            error: f.error,
        });
    }

    for doc in &docs {
        tracing::info!(id = %doc.id, title = %doc.title, "document ingested");
    }

    let succeeded: Vec<DocumentResponse> = docs.into_iter().map(DocumentResponse::from).collect();
    let status = if failed.is_empty() {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Ok((status, Json(BatchIngestResponse { succeeded, failed })))
}

async fn purge_document(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let mut store = state.store.write().await;
    match store
        .purge(id)
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        Some(doc) => {
            tracing::info!(%id, title = %doc.title, "document purged");
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(AppError::not_found("document not found")),
    }
}

async fn purge_documents(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BulkPurgeRequest>,
) -> ApiResult<(StatusCode, Json<BatchPurgeResponse>)> {
    if payload.ids.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(AppError::new("ids must not be empty")),
        ));
    }

    let mut store = state.store.write().await;
    let result = store.purge_many(payload.ids);
    drop(store);

    let purged: Vec<String> = result.purged.iter().map(|id| id.to_string()).collect();
    let failed: Vec<FailedItem> = result
        .failed
        .into_iter()
        .map(|f| FailedItem {
            name: f.id.to_string(),
            error: f.error,
        })
        .collect();

    for id in &purged {
        tracing::info!(%id, "document purged");
    }

    Ok((StatusCode::OK, Json(BatchPurgeResponse { purged, failed })))
}
