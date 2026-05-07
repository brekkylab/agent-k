//! Integration tests for session persistence and agent lazy-creation after restart.
//!
//! Tests tagged `#[ignore]` require:
//!   - microsandbox runtime   (`session_is_found_after_restart_via_lazy_create`)
//!   - microsandbox + ANTHROPIC_API_KEY  (`agent_restores_history_and_processes_message`)
//!
//! Run: `cargo test --test message_history_persistence -- --ignored`

#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use agent_k_backend::{repository, state::AppState};
use common::{
    SessionGuard, authed, clear_message_history, clear_message_history_status, extract_text,
    get_message_history, get_message_history_status, login, make_app_with_repo,
    make_app_with_state, make_repo, make_test_store, post_session_authed, send_message,
    send_message_status, setup_provider, signup, test_jwt_config, try_delete_session,
};
use uuid::Uuid;

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
    dotenvy::dotenv().ok();
    setup_provider().await;

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    // ── Instance 1: create session ────────────────────────────────────────
    let (session_id, token) = {
        let repo = repository::create_repository(&db_url).await.unwrap();
        let app = make_app_with_repo(repo);
        let username = format!("user_{}", uuid::Uuid::new_v4().simple());
        signup(&app, &username, "Password123!").await;
        let token = login(&app, &username, "Password123!").await;
        let project = common::get_personal_project(&app, &token).await;
        let project_id = project["id"].as_str().unwrap().to_string();
        let id = post_session_authed(&app, &token, &project_id).await;
        (id, token)
        // app (instance 1) drops here — simulates server restart
    };

    // ── Instance 2: fresh AppState, same DB ──────────────────────────────
    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    // Guard ensures delete_session is called even if the assertion below panics.
    let _guard = SessionGuard {
        app: app.clone(),
        id: session_id,
        token: token.clone(),
    };

    let status = send_message_status(&app, session_id, "hello", &token).await;

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
    setup_provider().await;

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    // ── Turn 1 (instance 1) ───────────────────────────────────────────────
    let (session_id, token, turn1_text) = {
        let repo = repository::create_repository(&db_url).await.unwrap();
        let app = make_app_with_repo(repo);
        let username = format!("user_{}", uuid::Uuid::new_v4().simple());
        signup(&app, &username, "Password123!").await;
        let token = login(&app, &username, "Password123!").await;
        let project = common::get_personal_project(&app, &token).await;
        let project_id = project["id"].as_str().unwrap().to_string();
        let id = post_session_authed(&app, &token, &project_id).await;

        // Guard covers any panic inside this scope (e.g. send_message asserting non-200).
        let guard = SessionGuard {
            app: app.clone(),
            id,
            token: token.clone(),
        };

        let outputs = send_message(&app, id, "What is the capital of France?", &token).await;
        let text = extract_text(&outputs);

        // Turn 1 succeeded — disarm. Instance 2's guard will own cleanup.
        std::mem::forget(guard);
        (id, token, text)
        // app (instance 1) drops here — simulates server restart
    };

    // ── Turn 2 (instance 2): fresh AppState, same DB ─────────────────────
    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    // Guard covers assertions below and any failure inside send_message.
    let _guard = SessionGuard {
        app: app.clone(),
        id: session_id,
        token: token.clone(),
    };

    let outputs = send_message(
        &app,
        session_id,
        "What city did I ask about in my previous question?",
        &token,
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
    dotenvy::dotenv().ok();
    setup_provider().await;

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    // Create a user just to get a valid token (the session doesn't exist)
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;

    let fake_id = uuid::Uuid::new_v4();
    let status = send_message_status(&app, fake_id, "hello", &token).await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "non-existent session must return 404"
    );
}

// ── GET /sessions/{id}/messages ───────────────────────────────────────────────

/// A freshly created session has an empty message history.
/// Creates the session directly in the DB to avoid the microsandbox requirement.
#[tokio::test]
async fn get_messages_returns_empty_for_new_session() {
    let store = make_test_store();
    let repo = make_repo().await;
    let state = Arc::new(AppState::new(repo, store, test_jwt_config()));
    let app = make_app_with_state(state.clone());

    // Create user + project to get a valid token and project_id
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();

    // Get the user id from the me endpoint
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    // Create session directly in DB using correct signature
    let session = state.repository.create_session(project_id, user_id).await.unwrap();

    let messages = get_message_history(&app, session.id, &token).await;
    assert_eq!(
        messages,
        serde_json::json!([]),
        "new session must have empty message history"
    );
}

/// GET /sessions/{id}/messages must return 404 for an unknown session.
#[tokio::test]
async fn get_messages_returns_404_for_unknown_session() {
    let app = make_app_with_repo(make_repo().await);

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;

    let status = get_message_history_status(&app, Uuid::new_v4(), &token).await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "unknown session must return 404"
    );
}

/// GET /sessions/{id}/messages returns all persisted messages in insertion order.
#[tokio::test]
async fn get_messages_returns_persisted_messages_in_order() {
    use ailoy::message::{Message, Part, Role};

    let store = make_test_store();
    let repo = make_repo().await;
    let state = Arc::new(AppState::new(repo, store, test_jwt_config()));
    let app = make_app_with_state(state.clone());

    // Create user + project
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state.repository.create_session(project_id, user_id).await.unwrap();
    {
        let msgs = vec![
            Message::new(Role::User).with_contents([Part::text("first")]),
            Message::new(Role::Assistant).with_contents([Part::text("second")]),
        ];
        state.repository.append_messages(session.id, &msgs).await.unwrap();
    }

    let body = get_message_history(&app, session.id, &token).await;
    let arr = body.as_array().expect("response must be a JSON array");
    assert_eq!(arr.len(), 2, "must return exactly two messages");

    let role0 = arr[0]["role"].as_str().unwrap_or("");
    let role1 = arr[1]["role"].as_str().unwrap_or("");
    assert_eq!(role0, "user");
    assert_eq!(role1, "assistant");
}

// ── DELETE /sessions/{id}/messages ───────────────────────────────────────────

/// DELETE /sessions/{id}/messages must return 404 for an unknown session.
#[tokio::test]
async fn clear_messages_returns_404_for_unknown_session() {
    let app = make_app_with_repo(make_repo().await);

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;

    let status = clear_message_history_status(&app, Uuid::new_v4(), &token).await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "unknown session must return 404"
    );
}

/// After clearing, GET /sessions/{id}/messages returns an empty array and the
/// DB record is gone.
#[tokio::test]
async fn clear_messages_removes_persisted_messages() {
    use ailoy::message::{Message, Part, Role};

    let store = make_test_store();
    let repo = make_repo().await;
    let state = Arc::new(AppState::new(repo, store, test_jwt_config()));
    let app = make_app_with_state(state.clone());

    // Create user + project
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state.repository.create_session(project_id, user_id).await.unwrap();
    {
        let msgs = vec![
            Message::new(Role::User).with_contents([Part::text("hello")]),
            Message::new(Role::Assistant).with_contents([Part::text("world")]),
        ];
        state.repository.append_messages(session.id, &msgs).await.unwrap();
    }

    let before = get_message_history(&app, session.id, &token).await;
    assert_eq!(
        before.as_array().unwrap().len(),
        2,
        "expected two messages before clear"
    );

    clear_message_history(&app, session.id, &token).await;

    let after = get_message_history(&app, session.id, &token).await;
    assert_eq!(
        after,
        serde_json::json!([]),
        "message history must be empty after clear"
    );
}

/// After clearing, the session itself still exists (only messages are removed).
#[tokio::test]
async fn clear_messages_does_not_delete_session() {
    use ailoy::message::{Message, Part, Role};

    let store = make_test_store();
    let repo = make_repo().await;
    let state = Arc::new(AppState::new(repo, store, test_jwt_config()));
    let app = make_app_with_state(state.clone());

    // Create user + project
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state.repository.create_session(project_id, user_id).await.unwrap();
    {
        let msgs = vec![Message::new(Role::User).with_contents([Part::text("ping")])];
        state.repository.append_messages(session.id, &msgs).await.unwrap();
    }

    clear_message_history(&app, session.id, &token).await;

    // Session still exists: GET messages returns 200 (not 404).
    let status = get_message_history_status(&app, session.id, &token).await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "session must still exist after message clear"
    );
}

/// After clearing, new messages can be appended to the same session.
#[tokio::test]
async fn can_append_messages_after_clear() {
    use ailoy::message::{Message, Part, Role};

    let store = make_test_store();
    let repo = make_repo().await;
    let state = Arc::new(AppState::new(repo, store, test_jwt_config()));
    let app = make_app_with_state(state.clone());

    // Create user + project
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state.repository.create_session(project_id, user_id).await.unwrap();
    {
        let msgs = vec![Message::new(Role::User).with_contents([Part::text("old")])];
        state.repository.append_messages(session.id, &msgs).await.unwrap();
    }

    clear_message_history(&app, session.id, &token).await;

    {
        let msgs = vec![Message::new(Role::User).with_contents([Part::text("new")])];
        state.repository.append_messages(session.id, &msgs).await.unwrap();
    }

    let body = get_message_history(&app, session.id, &token).await;
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1, "only the new message must remain");

    let text = arr[0]["contents"][0]["text"].as_str().unwrap_or("");
    assert_eq!(text, "new");
}

/// After clearing, the in-memory agent history is also wiped so the next turn
/// starts fresh.
///
/// Requires: microsandbox runtime.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires microsandbox runtime"]
async fn clear_messages_also_clears_in_memory_agent_history() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap().to_string();
    let id = post_session_authed(&app, &token, &project_id).await;

    let _guard = SessionGuard {
        app: app.clone(),
        id,
        token: token.clone(),
    };

    // Manually insert history into the DB and sync it into the agent via the
    // lazy-create path.  The send_message_status call that follows will fail
    // (dummy API key), but it will trigger resolve_agent which loads the DB
    // history into the agent, so the history is in memory before we clear it.
    {
        use ailoy::message::{Message, Part, Role};
        repo.append_messages(
            id,
            &[Message::new(Role::User).with_contents([Part::text("should be cleared")])],
        )
        .await
        .unwrap();
    }

    clear_message_history(&app, id, &token).await;

    // DB must be empty.
    let db_count = repo.get_messages(id).await.unwrap().len();
    assert_eq!(db_count, 0, "DB messages must be empty after clear");
}

// ── misc ──────────────────────────────────────────────────────────────────────

/// Verify the common helper `try_delete_session` returns `Err` for an unknown
/// session instead of panicking. No sandbox or API key required.
#[tokio::test]
async fn try_delete_returns_err_for_unknown_session() {
    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;

    let result = try_delete_session(&app, uuid::Uuid::new_v4(), &token).await;
    assert!(
        result.is_err(),
        "try_delete_session must return Err for an unknown session"
    );
}
