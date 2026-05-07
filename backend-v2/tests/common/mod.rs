//! Shared test helpers for backend-v2 integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use agent_k_backend::{auth::JwtConfig, repository, router, state::AppState};
use aide::openapi::OpenApi;
use ailoy::{agent::default_provider_mut, lang_model::LangModelProvider, tool::ToolProvider};
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use speedwagon::Store;
use tokio::sync::RwLock;
use tower::ServiceExt;

// ── Provider setup ────────────────────────────────────────────────────────────

pub async fn setup_provider() {
    let mut provider = default_provider_mut().await;
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
    provider.tools = ToolProvider::new().bash().python_repl().web_search();
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

pub fn make_test_store() -> speedwagon::SharedStore {
    let store_path = std::env::temp_dir().join(format!("speedwagon-test-{}", uuid::Uuid::new_v4()));
    Arc::new(RwLock::new(
        Store::new(store_path).expect("test store init"),
    ))
}

pub fn make_app_with_repo(repo: repository::AppRepository) -> axum::Router {
    let store = make_test_store();
    let state = Arc::new(AppState::new(repo, store, test_jwt_config()));
    make_app_with_state(state)
}

pub fn make_app_with_state(state: Arc<AppState>) -> axum::Router {
    router::get_router(state).finish_api(&mut OpenApi::default())
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
    assert_eq!(status, axum::http::StatusCode::OK, "list_projects failed: {body}");
    body["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|p| p["name"] == "Personal")
        .cloned()
        .expect("Personal project not found")
}

pub async fn post_session_authed(
    app: &axum::Router,
    token: &str,
    project_id: &str,
) -> uuid::Uuid {
    let (status, body) = authed(
        app,
        "POST",
        &format!("/projects/{project_id}/sessions"),
        token,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::CREATED,
        "post_session_authed failed: {body}"
    );
    uuid::Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
}

pub async fn add_member(
    app: &axum::Router,
    owner_token: &str,
    project_id: &str,
    username: &str,
) {
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
    let project_id = project["id"].as_str().unwrap().to_string();
    post_session_authed(app, &token, &project_id).await
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

pub async fn send_message(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
    token: &str,
) -> serde_json::Value {
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
        axum::http::StatusCode::OK,
        "send_message returned non-200 for session {id}"
    );
    value
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

pub async fn send_message_stream(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
    token: &str,
) -> Vec<serde_json::Value> {
    let body = serde_json::json!({ "content": content }).to_string();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/sessions/{id}/messages/stream"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "POST /sessions/{id}/messages/stream failed: {}",
        String::from_utf8_lossy(&bytes)
    );
    parse_sse_message_events(&bytes)
}

pub fn parse_sse_message_events(body: &[u8]) -> Vec<serde_json::Value> {
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
            if event_type != "message" {
                return None;
            }
            serde_json::from_str(data_line).ok()
        })
        .collect()
}

pub async fn get_message_history(
    app: &axum::Router,
    id: uuid::Uuid,
    token: &str,
) -> serde_json::Value {
    let (status, value) = authed(
        app,
        "GET",
        &format!("/sessions/{id}/messages"),
        token,
        None,
    )
    .await;
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
    let (status, _) = authed(
        app,
        "GET",
        &format!("/sessions/{id}/messages"),
        token,
        None,
    )
    .await;
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
