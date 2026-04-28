//! Integration tests for session persistence and agent lazy-creation after restart.
//!
//! Tests tagged `#[ignore]` require:
//!   - microsandbox runtime   (`session_is_found_after_restart_via_lazy_create`)
//!   - microsandbox + ANTHROPIC_API_KEY  (`agent_restores_history_and_processes_message`)
//!
//! Run: `cargo test --test message_history_persistence -- --ignored`

#[path = "common/mod.rs"]
mod common;

use agent_k_backend::repository;
use common::{
    SessionGuard, extract_text, make_app_with_repo, post_session, send_message,
    send_message_status, try_delete_session,
};

// ── tests ─────────────────────────────────────────────────────────────────────

/// After a simulated restart (new AppState, same DB), sending a message to an
/// existing session returns non-404 because the agent is lazy-created from the
/// persisted session record.
///
/// The `SessionGuard` registered on instance-2's app ensures the session is
/// deleted whether the assertion passes or panics.
///
/// Requires: microsandbox runtime.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires microsandbox runtime"]
async fn session_is_found_after_restart_via_lazy_create() {
    // Dummy key: validated only when agent.run() reaches the Anthropic API,
    // which this test never does (it only checks the HTTP status code).
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "dummy-key-for-lazy-create-test");
    }

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    // ── Instance 1: create session ────────────────────────────────────────
    let session_id = {
        let repo = repository::create_repository(&db_url).await.unwrap();
        let app = make_app_with_repo(repo);
        post_session(&app).await
        // app (instance 1) drops here — simulates server restart
    };

    // ── Instance 2: fresh AppState, same DB ──────────────────────────────
    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    // Guard ensures delete_session is called even if the assertion below panics.
    let _guard = SessionGuard {
        app: app.clone(),
        id: session_id,
    };

    let status = send_message_status(&app, session_id, "hello").await;

    assert_ne!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "expected lazy-create to handle the known session, but got 404"
    );
    // _guard drops here (or on panic above) → DELETE /sessions/{session_id}
}

/// Full end-to-end: agent processes turn 1, server restarts, agent is
/// lazy-created with turn-1 history, then answers a follow-up that requires
/// that history.
///
/// `SessionGuard` is registered twice:
///   - in instance 1's scope, armed for the turn-1 step; disarmed with
///     `mem::forget` on success so cleanup falls through to instance 2.
///   - in instance 2's scope, active for assertions and always fires on exit.
///
/// Requires: microsandbox runtime + ANTHROPIC_API_KEY (real value).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires microsandbox + ANTHROPIC_API_KEY"]
async fn agent_restores_history_and_processes_message() {
    dotenvy::dotenv().ok();

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    // ── Turn 1 (instance 1) ───────────────────────────────────────────────
    let (session_id, turn1_text) = {
        let repo = repository::create_repository(&db_url).await.unwrap();
        let app = make_app_with_repo(repo);
        let id = post_session(&app).await;

        // Guard covers any panic inside this scope (e.g. send_message asserting non-200).
        let guard = SessionGuard {
            app: app.clone(),
            id,
        };

        let outputs = send_message(&app, id, "What is the capital of France?").await;
        let text = extract_text(&outputs);

        // Turn 1 succeeded — disarm. Instance 2's guard will own cleanup.
        std::mem::forget(guard);
        (id, text)
        // app (instance 1) drops here — simulates server restart
    };

    // ── Turn 2 (instance 2): fresh AppState, same DB ─────────────────────
    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    // Guard covers assertions below and any failure inside send_message.
    let _guard = SessionGuard {
        app: app.clone(),
        id: session_id,
    };

    let outputs = send_message(
        &app,
        session_id,
        "What city did I ask about in my previous question?",
    )
    .await;
    let turn2_text = extract_text(&outputs);

    // ── Assertions ────────────────────────────────────────────────────────
    // _guard fires here (or on earlier panic) → DELETE /sessions/{session_id}
    assert!(
        turn1_text.to_lowercase().contains("paris"),
        "expected 'Paris' in turn-1 response, got: {turn1_text:?}"
    );
    assert!(
        turn2_text.to_lowercase().contains("france") || turn2_text.to_lowercase().contains("paris"),
        "expected history-aware response referencing France/Paris, got: {turn2_text:?}"
    );
}

/// Unknown session ID must return 404 — no session is created, so no cleanup
/// guard is needed.
#[tokio::test]
async fn unknown_session_returns_404() {
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "dummy");
    }

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    let fake_id = uuid::Uuid::new_v4();
    let status = send_message_status(&app, fake_id, "hello").await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "non-existent session must return 404"
    );
}

/// Verify the common helper `try_delete_session` returns `Err` for an unknown
/// session instead of panicking. No sandbox or API key required.
#[tokio::test]
async fn try_delete_returns_err_for_unknown_session() {
    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    let result = try_delete_session(&app, uuid::Uuid::new_v4()).await;
    assert!(
        result.is_err(),
        "try_delete_session must return Err for an unknown session"
    );
}
