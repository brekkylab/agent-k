//! Shared test helpers for backend-v2 integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use agent_k::knowledge_base::Store;
use agent_k_backend::{
    authn::JwtConfig,
    repository::{self, DbSenderKind, NewSessionMessage},
    router,
    state::AppState,
};
use aide::openapi::OpenApi;
use ailoy::{agent::default_provider_mut, lang_model::LangModelProvider, tool::ToolProvider};
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tokio::sync::RwLock;
use tower::ServiceExt;

// ── Provider setup ────────────────────────────────────────────────────────────

pub async fn setup_provider() {
    let mut provider = default_provider_mut();
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        provider
            .models
            .insert("openai/*".into(), LangModelProvider::openai(key));
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        provider
            .models
            .insert("anthropic/*".into(), LangModelProvider::anthropic(key));
    }
    if let Ok(key) = std::env::var("GEMINI_API_KEY") {
        provider
            .models
            .insert("google/*".into(), LangModelProvider::gemini(key));
    }
    provider.tools = ToolProvider::new();
}

// ── App / state creation ──────────────────────────────────────────────────────

pub fn test_jwt_config() -> JwtConfig {
    JwtConfig::new("test-secret-key-for-tests", 604_800)
}

pub async fn make_repo() -> repository::AppRepository {
    repository::create_repository("sqlite::memory:")
        .await
        .unwrap()
}

pub fn make_test_store() -> agent_k::knowledge_base::SharedStore {
    let store_path = std::env::temp_dir().join(format!("agent-k-test-{}", uuid::Uuid::new_v4()));
    Arc::new(RwLock::new(
        Store::new(store_path).expect("test store init"),
    ))
}

pub async fn make_app_repo_state() -> (axum::Router, repository::AppRepository, Arc<AppState>) {
    let repo = make_repo().await;
    let data_root = std::env::temp_dir().join(format!("agent-k-test-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo.clone(), test_jwt_config(), data_root));
    let app = make_app_with_state(state.clone());
    (app, repo, state)
}

pub fn make_app_with_repo(repo: repository::AppRepository) -> axum::Router {
    let data_root = std::env::temp_dir().join(format!("agent-k-test-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo, test_jwt_config(), data_root));
    make_app_with_state(state)
}

pub fn make_app_with_state(state: Arc<AppState>) -> axum::Router {
    router::get_router(state).finish_api(&mut OpenApi::default())
}

/// Creates an in-memory repo seeded with a single admin user, builds the app,
/// and logs in as that admin. Returns (app, admin_token, admin_id).
pub async fn make_admin_app() -> (axum::Router, String, uuid::Uuid) {
    use agent_k_backend::{authn, repository::NewUser};
    let repo = make_repo().await;
    let admin_id = uuid::Uuid::new_v4();
    let password_hash = authn::hash_password("adminpass1").unwrap();
    repo.create_user(NewUser {
        id: admin_id,
        username: "admin".to_string(),
        password_hash,
        role: authn::Role::Admin,
        display_name: None,
        is_active: true,
        preferred_language: "en".to_string(),
    })
    .await
    .unwrap();
    let app = make_app_with_repo(repo);
    let token = login(&app, "admin", "adminpass1").await;
    (app, token, admin_id)
}

// ── Auth helpers ──────────────────────────────────────────────────────────────

pub async fn signup_status(
    app: &axum::Router,
    username: &str,
    password: &str,
    display_name: Option<&str>,
) -> (axum::http::StatusCode, serde_json::Value) {
    let mut body = serde_json::json!({ "username": username, "password": password });
    if let Some(dn) = display_name {
        body["display_name"] = serde_json::Value::String(dn.to_string());
    }
    let req = Request::builder()
        .method("POST")
        .uri("/auth/signup")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

pub async fn signup(app: &axum::Router, username: &str, password: &str) -> serde_json::Value {
    let (status, body) = signup_status(app, username, password, None).await;
    assert_eq!(
        status,
        axum::http::StatusCode::CREATED,
        "signup failed: {body}"
    );
    body
}

pub async fn login_status(
    app: &axum::Router,
    username: &str,
    password: &str,
) -> (axum::http::StatusCode, serde_json::Value) {
    let body = serde_json::json!({ "username": username, "password": password });
    let req = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

pub async fn login(app: &axum::Router, username: &str, password: &str) -> String {
    let (status, body) = login_status(app, username, password).await;
    assert_eq!(status, axum::http::StatusCode::OK, "login failed: {body}");
    body["access_token"].as_str().unwrap().to_string()
}

pub async fn authed(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (axum::http::StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));

    let req_body = if let Some(b) = body {
        builder = builder.header("content-type", "application/json");
        Body::from(b.to_string())
    } else {
        Body::empty()
    };

    let resp = app
        .clone()
        .oneshot(builder.body(req_body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

// ── Project helpers ───────────────────────────────────────────────────────────

pub async fn get_personal_project(app: &axum::Router, token: &str) -> serde_json::Value {
    let (status, body) = authed(app, "GET", "/projects", token, None).await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "list_projects failed: {body}"
    );
    body["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|p| p["name"] == "Personal")
        .cloned()
        .expect("Personal project not found")
}

pub async fn post_session_authed(app: &axum::Router, token: &str, project_id: &str) -> uuid::Uuid {
    let (status, body) = authed(
        app,
        "POST",
        "/sessions",
        token,
        Some(serde_json::json!({ "project_ref": project_id })),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::CREATED,
        "post_session_authed failed: {body}"
    );
    uuid::Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
}

pub async fn add_member(app: &axum::Router, owner_token: &str, project_id: &str, username: &str) {
    let (status, body) = authed(
        app,
        "POST",
        &format!("/projects/{project_id}/members"),
        owner_token,
        Some(serde_json::json!({ "username": username })),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::NO_CONTENT,
        "add_member failed: {body}"
    );
}

pub async fn update_share_mode(
    app: &axum::Router,
    token: &str,
    session_id: uuid::Uuid,
    mode: &str,
) {
    let (status, body) = authed(
        app,
        "PATCH",
        &format!("/sessions/{session_id}"),
        token,
        Some(serde_json::json!({ "share_mode": mode })),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "update_share_mode failed: {body}"
    );
}

// ── Session helpers ───────────────────────────────────────────────────────────

/// Creates a throwaway user, logs in, and creates a session under their Personal project.
/// Use for tests that don't care about auth identity.
pub async fn post_session(app: &axum::Router) -> uuid::Uuid {
    let username = format!("testuser_{}", uuid::Uuid::new_v4().simple());
    signup(app, &username, "Password123!").await;
    let token = login(app, &username, "Password123!").await;
    let project = get_personal_project(app, &token).await;
    let project_slug = project["slug"].as_str().unwrap().to_string();
    post_session_authed(app, &token, &project_slug).await
}

pub async fn try_delete_session(
    app: &axum::Router,
    id: uuid::Uuid,
    token: &str,
) -> Result<(), String> {
    let (status, _body) = authed(app, "DELETE", &format!("/sessions/{id}"), token, None).await;
    if status != axum::http::StatusCode::NO_CONTENT {
        return Err(format!("DELETE /sessions/{id} returned {status}"));
    }
    Ok(())
}

pub async fn delete_session(app: &axum::Router, id: uuid::Uuid, token: &str) {
    try_delete_session(app, id, token)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
}

/// Trigger an agent run and wait for it to complete, returning the newly
/// produced messages shaped like the legacy `Vec<MessageOutput>` response
/// (each item has a `message` field, consumed by `extract_text`).
///
/// New protocol: `POST /messages` is fire-and-forget and returns `202`. We
/// snapshot the history length before triggering, then poll
/// `GET /sessions/{id}/messages` until the count grows past the snapshot (the
/// run persists the user message plus one or more agent messages on
/// completion). Panics on timeout.
pub async fn send_message(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
    token: &str,
) -> serde_json::Value {
    send_message_and_wait(app, id, content, token, 30).await
}

pub async fn send_message_and_wait(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
    token: &str,
    timeout_secs: u64,
) -> serde_json::Value {
    // Snapshot history length before triggering the run.
    let before = get_message_history(app, id, token).await["items"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);

    let body = serde_json::json!({ "content": content });
    let (status, value) = authed(
        app,
        "POST",
        &format!("/sessions/{id}/messages"),
        token,
        Some(body),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::ACCEPTED,
        "send_message returned non-202 for session {id}: {value}"
    );

    // Poll history until the run completes (count grows past the snapshot,
    // including the persisted user message + agent reply).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        let history = get_message_history(app, id, token).await;
        let items = history["items"].as_array().cloned().unwrap_or_default();
        if items.len() > before {
            // Wait for at least one new item — user message is persisted atomically
            // with agent messages on run completion. Return only the newly produced
            // items (drop the prior history), shaped like the legacy outputs for `extract_text`.
            return serde_json::Value::Array(items.into_iter().skip(before).collect());
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("send_message_and_wait timed out after {timeout_secs}s for session {id}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

pub async fn send_message_status(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
    token: &str,
) -> axum::http::StatusCode {
    let body = serde_json::json!({ "content": content });
    let (status, _) = authed(
        app,
        "POST",
        &format!("/sessions/{id}/messages"),
        token,
        Some(body),
    )
    .await;
    status
}

/// Legacy streaming helper. The `/messages/stream` endpoint was removed in
/// favour of the fire-and-forget trigger + WebSocket protocol. This now simply
/// triggers a run and waits for completion, returning the produced messages so
/// existing call sites that feed `extract_text_from_slice` keep working.
pub async fn send_message_stream(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
    token: &str,
) -> Vec<serde_json::Value> {
    send_message(app, id, content, token)
        .await
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Legacy raw-stream helper. Triggers an agent run (now returns `202`) and
/// returns immediately. Callers that previously consumed SSE bytes only relied
/// on the trigger side effect (e.g. WebSocket broadcasts), so no body is
/// returned.
pub async fn send_message_stream_raw(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
    token: &str,
) -> bytes::Bytes {
    let status = send_message_status(app, id, content, token).await;
    assert_eq!(
        status,
        axum::http::StatusCode::ACCEPTED,
        "POST /sessions/{id}/messages trigger failed with status {status}"
    );
    bytes::Bytes::new()
}

pub fn parse_sse_message_events(body: &[u8]) -> Vec<serde_json::Value> {
    parse_sse_events_by_type(body, "message")
        .into_iter()
        .filter_map(|data| serde_json::from_str(&data).ok())
        .collect()
}

/// Return the raw data strings for all SSE frames of the given event type.
pub fn parse_sse_events_by_type(body: &[u8], target_event: &str) -> Vec<String> {
    String::from_utf8_lossy(body)
        .split("\n\n")
        .filter(|s| !s.trim().is_empty())
        .filter_map(|chunk| {
            let mut event_type = "";
            let mut data_line = "";
            for line in chunk.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    event_type = v;
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data_line = v;
                }
            }
            if event_type == target_event {
                Some(data_line.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Converts `&[Message]` to `Vec<NewSessionMessage>` for use in tests.
/// `user_id` is set as `sender_user_id` for user-role messages.
pub fn to_new_msgs(
    msgs: &[ailoy::message::Message],
    user_id: uuid::Uuid,
) -> Vec<NewSessionMessage> {
    msgs.iter()
        .map(|m| {
            let (sender_kind, sender_name, sender_user_id) = match m.role {
                ailoy::message::Role::User => (DbSenderKind::User, None, Some(user_id)),
                _ => (DbSenderKind::Agent, Some("agent-k".to_string()), None),
            };
            NewSessionMessage {
                message: m.clone(),
                sender_kind,
                sender_name,
                sender_user_id,
                attachments: vec![],
                artifacts: vec![],
            }
        })
        .collect()
}

pub async fn get_message_history(
    app: &axum::Router,
    id: uuid::Uuid,
    token: &str,
) -> serde_json::Value {
    let (status, value) =
        authed(app, "GET", &format!("/sessions/{id}/messages"), token, None).await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "GET /sessions/{id}/messages failed"
    );
    value
}

pub async fn get_message_history_status(
    app: &axum::Router,
    id: uuid::Uuid,
    token: &str,
) -> axum::http::StatusCode {
    let (status, _) = authed(app, "GET", &format!("/sessions/{id}/messages"), token, None).await;
    status
}

pub async fn clear_message_history(app: &axum::Router, id: uuid::Uuid, token: &str) {
    let (status, _) = authed(
        app,
        "DELETE",
        &format!("/sessions/{id}/messages"),
        token,
        None,
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::NO_CONTENT,
        "DELETE /sessions/{id}/messages failed with status {status}"
    );
}

pub async fn clear_message_history_status(
    app: &axum::Router,
    id: uuid::Uuid,
    token: &str,
) -> axum::http::StatusCode {
    let (status, _) = authed(
        app,
        "DELETE",
        &format!("/sessions/{id}/messages"),
        token,
        None,
    )
    .await;
    status
}

// ── Multipart helper (shared by knowledge / e2e upload tests) ─────────────────

pub fn build_multipart_body(files: &[(&str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "----testboundary";
    let mut body = Vec::new();
    for (filename, content) in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (boundary.to_string(), body)
}

// ── Text extraction ───────────────────────────────────────────────────────────

pub fn extract_text_from_slice(outputs: &[serde_json::Value]) -> String {
    outputs
        .iter()
        .filter_map(|o| {
            let depth = o.get("depth").and_then(|d| d.as_u64()).unwrap_or(0);
            if depth != 0 {
                return None;
            }
            o.get("message")?
                .get("contents")?
                .as_array()?
                .iter()
                .filter_map(|p| p.get("text")?.as_str())
                .map(str::to_string)
                .reduce(|a, b| a + &b)
        })
        .collect::<Vec<_>>()
        .join("")
}

pub fn extract_text(outputs: &serde_json::Value) -> String {
    extract_text_from_slice(outputs.as_array().map(Vec::as_slice).unwrap_or(&[]))
}

// ── Dirent helpers ───────────────────────────────────────────────────────────

/// Upload files to a project's dirent store and assert all succeeded.
pub async fn upload_dirents(
    app: &axum::Router,
    token: &str,
    project_id: &str,
    files: &[(&str, &[u8])],
) -> serde_json::Value {
    let (boundary, body) = build_multipart_body(files);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/dirents?path=projects/{project_id}/shared"))
        .header("authorization", format!("Bearer {token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::OK,
        "dirent upload failed"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        value["failed"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(false),
        "dirent upload had failures: {value}"
    );
    value
}

// ── SessionGuard ──────────────────────────────────────────────────────────────

pub struct SessionGuard {
    pub app: axum::Router,
    pub id: uuid::Uuid,
    pub token: String,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let app = self.app.clone();
        let id = self.id;
        let token = self.token.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                if let Err(e) = try_delete_session(&app, id, &token).await {
                    eprintln!("SessionGuard: cleanup of {id} failed: {e}");
                }
            });
        });
    }
}
