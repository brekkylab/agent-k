//! Integration tests for per-session sandbox isolation, bash-tool execution, and streaming.
//!
//! Tests tagged `#[ignore]` require microsandbox + a real ANTHROPIC_API_KEY.
//!
//! Run all: `cargo test --test session_sandbox_test -- --ignored`

#[path = "common/mod.rs"]
mod common;

use std::{path::Path, sync::Arc};

use agent_k_backend::state::AppState;
use common::{
    delete_session, extract_text, extract_text_from_slice, get_personal_project, login, make_repo,
    make_test_store, post_session_authed, send_message, send_message_stream, setup_provider,
    signup, test_jwt_config, upload_dirents,
};

// ── helpers ───────────────────────────────────────────────────────────────────

async fn make_state() -> Arc<AppState> {
    let store = make_test_store();
    let data_root = std::env::temp_dir().join(format!("agent-k-sandbox-{}", uuid::Uuid::new_v4()));
    Arc::new(AppState::new(
        make_repo().await,
        store,
        test_jwt_config(),
        data_root,
    ))
}

// ── sandbox isolation ─────────────────────────────────────────────────────────

/// Two sessions must each get their own sandbox: a file written in session 1
/// must not be readable in session 2.
#[tokio::test]
async fn two_sessions_get_isolated_sandboxes() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let (app, _repo, state) = common::make_app_repo_state().await;

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap();

    let id1 = post_session_authed(&app, &token, project_id).await;
    let id2 = post_session_authed(&app, &token, project_id).await;
    assert_ne!(id1, id2, "two sessions must have different ids");

    let (re1, re2) = {
        let a1 = state.get_agent(&id1).expect("session 1 not found");
        let a2 = state.get_agent(&id2).expect("session 2 not found");
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

    delete_session(&app, id1, &token).await;
    delete_session(&app, id2, &token).await;

    ailoy::runenv::remove_persisted("session-doesnotexist")
        .await
        .expect("remove_persisted must be idempotent");
}

// ── bash tool ─────────────────────────────────────────────────────────────────

/// The agent uses the bash tool to write a file inside the session sandbox,
/// then reads it back.
///
/// Requires: microsandbox runtime + ANTHROPIC_API_KEY (real value).
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn agent_writes_and_reads_file_via_bash_in_sandbox() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let state = make_state().await;
    let app = common::make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap();
    let id = post_session_authed(&app, &token, project_id).await;

    let outputs = send_message(
        &app,
        id,
        "Run the following bash command exactly and report its output: \
         echo 'sandbox_ok' > /workspace/probe.txt && cat /workspace/probe.txt",
        &token,
    )
    .await;

    let text = extract_text(&outputs);
    assert!(
        text.contains("sandbox_ok"),
        "expected 'sandbox_ok' in agent response, got: {text:?}"
    );

    let agent_arc = state.get_agent(&id).unwrap();
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

    delete_session(&app, id, &token).await;
}

/// Files uploaded via the dirent API must be readable by the agent inside the
/// session sandbox at `/workspace/.uploads/<path>`.
///
/// Requires: microsandbox runtime + ANTHROPIC_API_KEY (real value).
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn agent_can_read_uploaded_files_from_workspace_uploads() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let state = make_state().await;
    let app = common::make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap();

    upload_dirents(
        &app,
        &token,
        project_id,
        &[("context.txt", b"SENTINEL_UPLOAD_OK")],
    )
    .await;

    let session_id = post_session_authed(&app, &token, project_id).await;

    let outputs = send_message(
        &app,
        session_id,
        "Run this bash command exactly and report the output: cat /workspace/.uploads/context.txt",
        &token,
    )
    .await;

    let text = extract_text(&outputs);
    assert!(
        text.contains("SENTINEL_UPLOAD_OK"),
        "expected agent to read 'SENTINEL_UPLOAD_OK' from /workspace/.uploads/context.txt, got: {text:?}"
    );

    delete_session(&app, session_id, &token).await;
}

// ── streaming ─────────────────────────────────────────────────────────────────

/// Sending a non-streaming message to a non-existent session must return 404.
#[tokio::test]
async fn send_message_to_unknown_session_returns_404() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let state = make_state().await;
    let app = common::make_app_with_state(state);

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;

    let fake_id = uuid::Uuid::new_v4();
    let (status, _) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{fake_id}/messages"),
        &token,
        Some(serde_json::json!({ "content": "hi" })),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "non-streaming message to unknown session must return 404"
    );
}

/// Sending a stream request to a non-existent session must return 404.
#[tokio::test]
async fn stream_returns_404_for_unknown_session() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let state = make_state().await;
    let app = common::make_app_with_state(state);

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;

    let fake_id = uuid::Uuid::new_v4();
    let (status, _) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{fake_id}/messages/stream"),
        &token,
        Some(serde_json::json!({ "content": "hi" })),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
}

/// The streaming endpoint emits `event: message` SSE blocks and ends with
/// `event: done`. The agent uses bash to write/read a file in the sandbox.
///
/// Requires: microsandbox runtime + ANTHROPIC_API_KEY (real value).
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn agent_writes_and_reads_file_via_bash_streaming() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let state = make_state().await;
    let app = common::make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap();
    let id = post_session_authed(&app, &token, project_id).await;

    let events = send_message_stream(
        &app,
        id,
        "Run the following bash command exactly and report its output: \
         echo 'sandbox_ok' > /workspace/probe_stream.txt \
         && cat /workspace/probe_stream.txt",
        &token,
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

    let agent_arc = state.get_agent(&id).unwrap();
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

    delete_session(&app, id, &token).await;
}
