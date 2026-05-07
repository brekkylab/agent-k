//! Shared test helpers for backend-v2 integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use agent_k_backend::{repository, router, state::AppState};
use aide::openapi::OpenApi;
use ailoy::{agent::default_provider_mut, lang_model::LangModelProvider, tool::ToolProvider};
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use speedwagon::Store;
use tokio::sync::RwLock;
use tower::ServiceExt;

// ── Provider setup ────────────────────────────────────────────────────────────

/// Register all available API keys and basic builtin tools with the global
/// `default_provider`.  Call this once per test after `dotenvy::dotenv().ok()`.
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

/// In-memory SQLite repository — state does not survive across instances.
pub async fn make_repo() -> repository::AppRepository {
    repository::create_repository("sqlite::memory:")
        .await
        .unwrap()
}

/// Create a SharedStore + ToolSet backed by a temporary directory.
pub fn make_test_store() -> speedwagon::SharedStore {
    let store_path = std::env::temp_dir().join(format!("speedwagon-test-{}", uuid::Uuid::new_v4()));
    Arc::new(RwLock::new(
        Store::new(&store_path).expect("test store init"),
    ))
}

/// Build an app from an already-constructed repository.
pub fn make_app_with_repo(repo: repository::AppRepository) -> axum::Router {
    let store = make_test_store();
    let state = Arc::new(AppState::new(repo, store));
    make_app_with_state(state)
}

/// Build an app from an already-constructed state (useful when tests need to
/// inspect the state directly, e.g. to read agent internals).
pub fn make_app_with_state(state: Arc<AppState>) -> axum::Router {
    router::get_router(state).finish_api(&mut OpenApi::default())
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

pub async fn post_session(app: &axum::Router) -> uuid::Uuid {
    let req = Request::builder()
        .method("POST")
        .uri("/sessions")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        axum::http::StatusCode::CREATED,
        "POST /sessions failed: {}",
        String::from_utf8_lossy(&bytes)
    );
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    uuid::Uuid::parse_str(v["id"].as_str().unwrap()).unwrap()
}

/// Attempt to delete a session; returns `Err` instead of panicking.
/// Suitable for use inside `Drop` implementations.
pub async fn try_delete_session(app: &axum::Router, id: uuid::Uuid) -> Result<(), String> {
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/sessions/{id}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if status != axum::http::StatusCode::NO_CONTENT {
        return Err(format!("DELETE /sessions/{id} returned {status}"));
    }
    Ok(())
}

/// Delete a session and assert the response is 204.
pub async fn delete_session(app: &axum::Router, id: uuid::Uuid) {
    try_delete_session(app, id)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
}

/// Send a message and assert the response is 200. Returns the parsed body.
pub async fn send_message(app: &axum::Router, id: uuid::Uuid, content: &str) -> serde_json::Value {
    let body = serde_json::json!({ "content": content }).to_string();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/sessions/{id}/messages"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::OK,
        "send_message returned non-200 for session {id}"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Send a message and return only the HTTP status code (no assertion).
pub async fn send_message_status(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
) -> axum::http::StatusCode {
    let body = serde_json::json!({ "content": content }).to_string();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/sessions/{id}/messages"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    app.clone().oneshot(req).await.unwrap().status()
}

/// Send a message via the SSE streaming endpoint. Returns parsed `event:
/// message` payloads; `event: done` / `event: error` blocks are omitted.
pub async fn send_message_stream(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
) -> Vec<serde_json::Value> {
    let body = serde_json::json!({ "content": content }).to_string();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/sessions/{id}/messages/stream"))
        .header("content-type", "application/json")
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

/// Fetch message history for a session and assert 200. Returns the parsed body.
pub async fn get_message_history(app: &axum::Router, id: uuid::Uuid) -> serde_json::Value {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/sessions/{id}/messages"))
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "GET /sessions/{id}/messages failed: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

/// Fetch message history and return only the HTTP status code (no assertion).
pub async fn get_message_history_status(
    app: &axum::Router,
    id: uuid::Uuid,
) -> axum::http::StatusCode {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/sessions/{id}/messages"))
        .body(Body::empty())
        .unwrap();

    app.clone().oneshot(req).await.unwrap().status()
}

/// Clear message history for a session and assert 204.
pub async fn clear_message_history(app: &axum::Router, id: uuid::Uuid) {
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/sessions/{id}/messages"))
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    assert_eq!(
        status,
        axum::http::StatusCode::NO_CONTENT,
        "DELETE /sessions/{id}/messages failed with status {status}"
    );
}

/// Clear message history and return only the HTTP status code (no assertion).
pub async fn clear_message_history_status(
    app: &axum::Router,
    id: uuid::Uuid,
) -> axum::http::StatusCode {
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/sessions/{id}/messages"))
        .body(Body::empty())
        .unwrap();

    app.clone().oneshot(req).await.unwrap().status()
}

// ── Document helpers ─────────────────────────────────────────────────────────

pub async fn list_documents(app: &axum::Router) -> Vec<serde_json::Value> {
    let req = Request::builder()
        .method("GET")
        .uri("/documents")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn build_multipart_body(files: &[(&str, &[u8])]) -> (String, Vec<u8>) {
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

/// Ingest a single file and return the first succeeded document.
pub async fn ingest_document(
    app: &axum::Router,
    filename: &str,
    content: &[u8],
) -> serde_json::Value {
    let batch = ingest_documents(app, &[(filename, content)]).await;
    let succeeded = batch["succeeded"]
        .as_array()
        .expect("succeeded should be array");
    assert!(
        !succeeded.is_empty(),
        "ingest_document: no succeeded items — failed: {:?}",
        batch["failed"]
    );
    succeeded[0].clone()
}

/// Ingest multiple files and return the full BatchIngestResponse.
pub async fn ingest_documents(app: &axum::Router, files: &[(&str, &[u8])]) -> serde_json::Value {
    post_documents(app, files).await.1
}

/// Ingest files and also return the HTTP status code.
pub async fn ingest_documents_with_status(
    app: &axum::Router,
    files: &[(&str, &[u8])],
) -> (axum::http::StatusCode, serde_json::Value) {
    post_documents(app, files).await
}

async fn post_documents(
    app: &axum::Router,
    files: &[(&str, &[u8])],
) -> (axum::http::StatusCode, serde_json::Value) {
    let (boundary, body) = build_multipart_body(files);

    let req = Request::builder()
        .method("POST")
        .uri("/documents")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

pub async fn purge_document(app: &axum::Router, id: &str) -> axum::http::StatusCode {
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/documents/{id}"))
        .body(Body::empty())
        .unwrap();

    app.clone().oneshot(req).await.unwrap().status()
}

pub async fn bulk_purge_documents(
    app: &axum::Router,
    ids: &[&str],
) -> (axum::http::StatusCode, serde_json::Value) {
    let payload = serde_json::json!({ "ids": ids });
    let req = Request::builder()
        .method("DELETE")
        .uri("/documents")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

pub async fn get_document(
    app: &axum::Router,
    id: &str,
) -> (axum::http::StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/documents/{id}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

// ── Text extraction ───────────────────────────────────────────────────────────

/// Concatenate all text parts from depth-0 assistant messages in a slice.
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

/// Convenience wrapper over [`extract_text_from_slice`] for a `Value` array.
pub fn extract_text(outputs: &serde_json::Value) -> String {
    extract_text_from_slice(outputs.as_array().map(Vec::as_slice).unwrap_or(&[]))
}

// ── SessionGuard ──────────────────────────────────────────────────────────────

/// RAII guard that deletes a session when dropped — even on panic.
///
/// Uses [`tokio::task::block_in_place`] so the enclosing test must run on a
/// multi-thread runtime: `#[tokio::test(flavor = "multi_thread")]`.
pub struct SessionGuard {
    pub app: axum::Router,
    pub id: uuid::Uuid,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let app = self.app.clone();
        let id = self.id;
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                if let Err(e) = try_delete_session(&app, id).await {
                    eprintln!("SessionGuard: cleanup of {id} failed: {e}");
                }
            });
        });
    }
}
