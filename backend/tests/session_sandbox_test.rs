//! Integration tests for per-session sandbox isolation, bash-tool execution, and streaming.
//!
//! Tests tagged `#[ignore]` require microsandbox + a real ANTHROPIC_API_KEY.
//!
//! Run all: `cargo test --test session_sandbox_test -- --ignored`

#[path = "common/mod.rs"]
mod common;

use std::{path::Path, sync::Arc};

use agent_k::agents::GUEST_SHARED_DIR;
use agent_k_backend::state::AppState;
use ailoy::{
    agent::{AgentBuilder, default_provider_mut},
    lang_model::LangModelProvider,
};
use axum::http::StatusCode;
use common::{
    delete_session, extract_text, extract_text_from_slice, get_personal_project, login, make_repo,
    post_session_authed, send_message, send_message_stream, setup_provider,
    signup, test_jwt_config, upload_dirents,
};

fn ensure_test_provider() {
    let mut provider = default_provider_mut();
    provider.models.insert(
        "openai/*".into(),
        LangModelProvider::openai("fake-key-for-test".into()),
    );
}

// ── helpers ───────────────────────────────────────────────────────────────────

async fn make_state() -> Arc<AppState> {
    let data_root = std::env::temp_dir().join(format!("agent-k-sandbox-{}", uuid::Uuid::new_v4()));
    Arc::new(AppState::new(
        make_repo().await,
        test_jwt_config(),
        data_root,
    ))
}

// ── sandbox isolation ─────────────────────────────────────────────────────────

/// Two sessions must each get their own sandbox: a file written in session 1
/// must not be readable in session 2.
///
/// Skips gracefully if the microsandbox (Docker) is unavailable.
#[tokio::test]
async fn two_sessions_get_isolated_sandboxes() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let (app, _repo, state) = common::make_app_repo_state().await;

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_slug = project["slug"].as_str().unwrap();
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();

    let id1 = post_session_authed(&app, &token, project_slug).await;
    let id2 = post_session_authed(&app, &token, project_slug).await;
    assert_ne!(id1, id2, "two sessions must have different ids");

    // POST /sessions only writes a DB record; agents are built lazily on first
    // message.  Build them explicitly here so we can access their sandboxed
    // RunEnv without sending a real LLM message.
    // Skip gracefully if the microsandbox (Docker) is unavailable.
    let agent1 = match agent_k_backend::handlers::build_session_agent(&state, project_id, id1).await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("skipping: microsandbox unavailable: {e}");
            return;
        }
    };
    let agent2 = match agent_k_backend::handlers::build_session_agent(&state, project_id, id2).await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("skipping: microsandbox unavailable: {e}");
            return;
        }
    };
    state.insert_agent(id1, agent1);
    state.insert_agent(id2, agent2);

    let (re1, re2) = {
        let a1 = state.get_agent(&id1).expect("session 1 not found");
        let a2 = state.get_agent(&id2).expect("session 2 not found");
        let guard1 = a1.try_lock().expect("agent 1 locked unexpectedly");
        let guard2 = a2.try_lock().expect("agent 2 locked unexpectedly");
        (guard1.state.runenv.clone(), guard2.state.runenv.clone())
    };

    let h1 = re1.get().await.expect("session 1 runenv boot failed");
    let h2 = re2.get().await.expect("session 2 runenv boot failed");

    h1.write(Path::new("/workspace/iso.txt"), b"session1")
        .await
        .expect("write to session 1 runenv failed");

    let read_result = h2.read(Path::new("/workspace/iso.txt")).await;
    assert!(
        read_result.is_err(),
        "session 2 must not be able to read a file written in session 1's sandbox"
    );

    drop(h1);
    drop(h2);

    delete_session(&app, id1, &token).await;
    delete_session(&app, id2, &token).await;
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
    let project_slug = project["slug"].as_str().unwrap();
    let id = post_session_authed(&app, &token, project_slug).await;

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
    let handle = agent.state.runenv.get().await.expect("runenv boot failed");
    let contents = handle
        .read(Path::new("/workspace/probe.txt"))
        .await
        .expect("probe.txt must exist in sandbox after agent wrote it");
    assert!(
        contents.starts_with(b"sandbox_ok"),
        "file contents mismatch: {contents:?}"
    );
    drop(handle);
    drop(agent);

    delete_session(&app, id, &token).await;
}

/// Files uploaded via the dirent API to the project's shared scope must be
/// readable by the agent inside the session sandbox at `{GUEST_SHARED_DIR}/<path>`.
///
/// Requires: microsandbox runtime + ANTHROPIC_API_KEY (real value).
#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY"]
async fn agent_can_read_shared_files_from_shared_data() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let state = make_state().await;
    let app = common::make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_slug = project["slug"].as_str().unwrap();
    let project_id = project["id"].as_str().unwrap();

    upload_dirents(
        &app,
        &token,
        project_id,
        &[("context.txt", b"SENTINEL_UPLOAD_OK")],
    )
    .await;

    let session_id = post_session_authed(&app, &token, project_slug).await;

    let outputs = send_message(
        &app,
        session_id,
        &format!("Run this bash command exactly and report the output: cat {GUEST_SHARED_DIR}/context.txt"),
        &token,
    )
    .await;

    let text = extract_text(&outputs);
    assert!(
        text.contains("SENTINEL_UPLOAD_OK"),
        "expected agent to read 'SENTINEL_UPLOAD_OK' from {GUEST_SHARED_DIR}/context.txt, got: {text:?}"
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
    let project_slug = project["slug"].as_str().unwrap();
    let id = post_session_authed(&app, &token, project_slug).await;

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
    let handle = agent.state.runenv.get().await.expect("runenv boot failed");
    let contents = handle
        .read(Path::new("/workspace/probe_stream.txt"))
        .await
        .expect("probe_stream.txt must exist in sandbox");
    assert!(
        contents.starts_with(b"sandbox_ok"),
        "file contents mismatch: {contents:?}"
    );
    drop(handle);
    drop(agent);

    delete_session(&app, id, &token).await;
}

// ── concurrency ───────────────────────────────────────────────────────────────

/// While a session's agent is locked (simulating a run in progress), a second
/// POST /sessions/{id}/messages request must return 423 Locked immediately.
/// No LLM call is made — the test inserts a fake agent and holds its mutex.
#[tokio::test]
async fn concurrent_send_returns_423() {
    dotenvy::dotenv().ok();
    ensure_test_provider();

    let (app, repo, state) = common::make_app_repo_state().await;

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    let user_info = signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let user_id = uuid::Uuid::parse_str(user_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, user_id).await.unwrap();

    let agent = AgentBuilder::new("openai/gpt-4o-mini")
        .build()
        .expect("AgentBuilder::build() must succeed with a registered provider");
    state.insert_agent(session.id, agent);

    // Hold the agent lock — simulates an in-progress run.
    let _guard = state
        .get_agent(&session.id)
        .expect("agent must exist after insert")
        .try_lock_owned()
        .expect("freshly inserted agent must not be locked");

    let (status, _) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{}/messages", session.id),
        &token,
        Some(serde_json::json!({ "content": "hello" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::LOCKED,
        "POST /messages while agent is locked must return 423"
    );
}
