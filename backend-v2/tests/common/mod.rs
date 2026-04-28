//! Shared test helpers for backend-v2 integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use agent_k_backend::{repository, router, state::AppState};
use aide::openapi::OpenApi;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tokio::sync::Mutex;
use tower::ServiceExt;

// ── App / state creation ──────────────────────────────────────────────────────

/// In-memory SQLite repository — state does not survive across instances.
pub async fn make_repo() -> repository::AppRepository {
    repository::create_repository("sqlite::memory:")
        .await
        .unwrap()
}

/// Build an app from an already-constructed repository.
pub fn make_app_with_repo(repo: repository::AppRepository) -> axum::Router {
    let state = Arc::new(Mutex::new(AppState::new(repo)));
    make_app_with_state(state)
}

/// Build an app from an already-constructed state (useful when tests need to
/// inspect the state directly, e.g. to read agent internals).
pub fn make_app_with_state(state: Arc<Mutex<AppState>>) -> axum::Router {
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
    let resp = app
        .clone()
        .oneshot(req)
        .await
        .map_err(|e| e.to_string())?;
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
pub async fn send_message(
    app: &axum::Router,
    id: uuid::Uuid,
    content: &str,
) -> serde_json::Value {
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
