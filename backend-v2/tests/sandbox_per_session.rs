//! Integration tests for per-session sandbox isolation and bash-tool execution.
//!
//! All tests are `#[ignore]` by default because they require microsandbox
//! (auto-downloaded on first run) and/or a real ANTHROPIC_API_KEY.
//!
//! Run all: `cargo test --test sandbox_per_session -- --ignored`

#[path = "common/mod.rs"]
mod common;

use std::path::Path;
use std::sync::Arc;

use agent_k_backend::state::AppState;
use common::{
    delete_session, extract_text, extract_text_from_slice, make_app_with_state, make_repo,
    post_session, send_message, send_message_stream,
};
use tokio::sync::Mutex;

// ── helpers ───────────────────────────────────────────────────────────────────

async fn make_state() -> Arc<Mutex<AppState>> {
    Arc::new(Mutex::new(AppState::new(make_repo().await)))
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Two sessions must each get their own sandbox: a file written in session 1
/// must not be readable in session 2.
///
/// Requires: microsandbox runtime.
/// Does NOT require a real API key (agent.run() is never called here).
#[tokio::test]
#[ignore = "requires microsandbox; boots two VMs"]
async fn two_sessions_get_isolated_sandboxes() {
    if std::env::var("ANTHROPIC_API_KEY").is_err() {
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "dummy");
        }
    }

    let state = make_state().await;
    let app = make_app_with_state(state.clone());

    let id1 = post_session(&app).await;
    let id2 = post_session(&app).await;
    assert_ne!(id1, id2, "two sessions must have different ids");

    let (re1, re2) = {
        let st = state.lock().await;
        let a1 = st.get_agent(&id1).expect("session 1 not found");
        let a2 = st.get_agent(&id2).expect("session 2 not found");
        // Agents are not running now, so try_lock succeeds.
        let guard1 = a1.try_lock().expect("agent 1 locked unexpectedly");
        let guard2 = a2.try_lock().expect("agent 2 locked unexpectedly");
        (guard1.state.runenv.clone(), guard2.state.runenv.clone())
    };

    assert!(
        !Arc::ptr_eq(&re1, &re2),
        "session 1 and 2 must not share the same runenv Arc"
    );

    re1.write(Path::new("/workspace/iso.txt"), b"session1")
        .await
        .expect("write to session 1 runenv failed");

    let read_result = re2.read(Path::new("/workspace/iso.txt")).await;
    assert!(
        read_result.is_err(),
        "session 2 must not be able to read a file written in session 1's sandbox"
    );

    delete_session(&app, id1).await;
    delete_session(&app, id2).await;

    // remove_persisted is idempotent.
    ailoy::runenv::remove_persisted("session-doesnotexist")
        .await
        .expect("remove_persisted must be idempotent");
}

/// The agent uses the bash tool to write a file inside the session sandbox,
/// then reads it back.  Verifies that the bash tool is wired to the sandbox
/// and that the agent's response reflects the file contents.
///
/// Requires: microsandbox runtime + ANTHROPIC_API_KEY (real value).
#[tokio::test]
#[ignore = "requires microsandbox + ANTHROPIC_API_KEY"]
async fn agent_writes_and_reads_file_via_bash_in_sandbox() {
    dotenvy::dotenv().ok();

    let state = make_state().await;
    let app = make_app_with_state(state.clone());

    let id = post_session(&app).await;

    // Ask the agent to write a sentinel value and read it back.
    let outputs = send_message(
        &app,
        id,
        "Run the following bash command exactly and report its output: \
         echo 'sandbox_ok' > /workspace/probe.txt && cat /workspace/probe.txt",
    )
    .await;

    let text = extract_text(&outputs);
    assert!(
        text.contains("sandbox_ok"),
        "expected 'sandbox_ok' in agent response, got: {text:?}"
    );

    // Verify via runenv directly that the file exists in the sandbox.
    let agent_arc = state.lock().await.get_agent(&id).unwrap();
    let agent = agent_arc.lock().await;
    let contents = agent
        .state
        .runenv
        .read(Path::new("/workspace/probe.txt"))
        .await
        .expect("probe.txt must exist in sandbox after agent wrote it");
    assert!(
        contents.starts_with(b"sandbox_ok"),
        "file contents mismatch: {contents:?}"
    );
    drop(agent);

    delete_session(&app, id).await;
}

// ── streaming tests ───────────────────────────────────────────────────────────

/// Sending a stream request to a non-existent session must return 404.
/// Does not require microsandbox or an API key.
#[tokio::test]
async fn stream_returns_404_for_unknown_session() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    if std::env::var("ANTHROPIC_API_KEY").is_err() {
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "dummy");
        }
    }

    let state = make_state().await;
    let app = make_app_with_state(state);

    let fake_id = uuid::Uuid::new_v4();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/sessions/{fake_id}/messages/stream"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"content":"hi"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

/// The streaming endpoint emits `event: message` SSE blocks and ends with
/// `event: done`.  The agent uses the bash tool to write/read a file in the
/// sandbox, and the streamed response must contain "sandbox_ok".
///
/// Requires: microsandbox runtime + ANTHROPIC_API_KEY (real value).
#[tokio::test]
#[ignore = "requires microsandbox + ANTHROPIC_API_KEY"]
async fn agent_writes_and_reads_file_via_bash_streaming() {
    dotenvy::dotenv().ok();

    let state = make_state().await;
    let app = make_app_with_state(state.clone());

    let id = post_session(&app).await;

    let events = send_message_stream(
        &app,
        id,
        "Run the following bash command exactly and report its output: \
         echo 'sandbox_ok' > /workspace/probe_stream.txt \
         && cat /workspace/probe_stream.txt",
    )
    .await;

    assert!(
        !events.is_empty(),
        "SSE stream must emit at least one message event"
    );

    let text = extract_text_from_slice(&events);
    assert!(
        text.contains("sandbox_ok"),
        "expected 'sandbox_ok' in streamed response, got: {text:?}"
    );

    // Verify the file persisted in the sandbox after the stream ended.
    let agent_arc = state.lock().await.get_agent(&id).unwrap();
    let agent = agent_arc.lock().await;
    let contents = agent
        .state
        .runenv
        .read(Path::new("/workspace/probe_stream.txt"))
        .await
        .expect("probe_stream.txt must exist in sandbox");
    assert!(
        contents.starts_with(b"sandbox_ok"),
        "file contents mismatch: {contents:?}"
    );
    drop(agent);

    delete_session(&app, id).await;
}
