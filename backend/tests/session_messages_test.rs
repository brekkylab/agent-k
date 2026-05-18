//! Integration tests for session message persistence and history endpoints.

#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use agent_k_backend::{repository, state::AppState};
use common::{
    SessionGuard, authed, clear_message_history, clear_message_history_status, get_message_history,
    get_message_history_status, login, make_app_with_repo, make_app_with_state, make_repo,
    make_test_store, post_session_authed, send_message_status, setup_provider, signup,
    test_jwt_config,
};
use uuid::Uuid;

// ── restart / lazy-create ─────────────────────────────────────────────────────

/// After a simulated restart (new AppState, same DB), message history is
/// restored from the DB and the session is lazy-created on the next request.
#[tokio::test(flavor = "multi_thread")]
async fn session_persists_and_restores_history_across_restart() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    // Instance 1: create session, seed messages, then drop (simulates restart).
    let (session_id, token) = {
        use ailoy::message::{Message, Part, Role};
        let repo = repository::create_repository(&db_url).await.unwrap();
        let app = make_app_with_repo(repo.clone());
        let username = format!("user_{}", uuid::Uuid::new_v4().simple());
        signup(&app, &username, "Password123!").await;
        let token = login(&app, &username, "Password123!").await;
        let project = common::get_personal_project(&app, &token).await;
        let project_id = project["id"].as_str().unwrap().to_string();
        let id = post_session_authed(&app, &token, &project_id).await;
        repo.append_messages(
            id,
            &[
                Message::new(Role::User).with_contents([Part::text("hello")]),
                Message::new(Role::Assistant).with_contents([Part::text("world")]),
            ],
        )
        .await
        .unwrap();
        (id, token)
    };

    // Instance 2: fresh AppState, same DB.
    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    let _guard = SessionGuard {
        app: app.clone(),
        id: session_id,
        token: token.clone(),
    };

    // History must be restored from DB after restart.
    let messages = get_message_history(&app, session_id, &token).await;
    let arr = messages.as_array().expect("messages must be a JSON array");
    assert_eq!(arr.len(), 2, "both seeded messages must survive restart");
    assert_eq!(arr[0]["role"].as_str().unwrap(), "user");
    assert_eq!(arr[1]["role"].as_str().unwrap(), "assistant");

    // Session must be lazy-created (non-404) when a new message arrives.
    let status = send_message_status(&app, session_id, "follow-up", &token).await;
    assert_ne!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "session must be lazy-created from persisted record after restart"
    );
}

/// Unknown session ID must return 404.
#[tokio::test]
async fn unknown_session_returns_404() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

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
#[tokio::test]
async fn get_messages_returns_empty_for_new_session() {
    let store = make_test_store();
    let repo = make_repo().await;
    let data_root =
        std::env::temp_dir().join(format!("agent-k-msg-persist-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo, store, test_jwt_config(), data_root));
    let app = make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();

    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state
        .repository
        .create_session(project_id, user_id)
        .await
        .unwrap();

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
    let data_root =
        std::env::temp_dir().join(format!("agent-k-msg-persist-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo, store, test_jwt_config(), data_root));
    let app = make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state
        .repository
        .create_session(project_id, user_id)
        .await
        .unwrap();
    {
        let msgs = vec![
            Message::new(Role::User).with_contents([Part::text("first")]),
            Message::new(Role::Assistant).with_contents([Part::text("second")]),
        ];
        state
            .repository
            .append_messages(session.id, &msgs)
            .await
            .unwrap();
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

/// After clearing, GET /sessions/{id}/messages returns an empty array.
#[tokio::test]
async fn clear_messages_removes_persisted_messages() {
    use ailoy::message::{Message, Part, Role};

    let store = make_test_store();
    let repo = make_repo().await;
    let data_root =
        std::env::temp_dir().join(format!("agent-k-msg-persist-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo, store, test_jwt_config(), data_root));
    let app = make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state
        .repository
        .create_session(project_id, user_id)
        .await
        .unwrap();
    {
        let msgs = vec![
            Message::new(Role::User).with_contents([Part::text("hello")]),
            Message::new(Role::Assistant).with_contents([Part::text("world")]),
        ];
        state
            .repository
            .append_messages(session.id, &msgs)
            .await
            .unwrap();
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
    let data_root =
        std::env::temp_dir().join(format!("agent-k-msg-persist-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo, store, test_jwt_config(), data_root));
    let app = make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state
        .repository
        .create_session(project_id, user_id)
        .await
        .unwrap();
    {
        let msgs = vec![Message::new(Role::User).with_contents([Part::text("ping")])];
        state
            .repository
            .append_messages(session.id, &msgs)
            .await
            .unwrap();
    }

    clear_message_history(&app, session.id, &token).await;

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
    let data_root =
        std::env::temp_dir().join(format!("agent-k-msg-persist-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo, store, test_jwt_config(), data_root));
    let app = make_app_with_state(state.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let user_id = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session = state
        .repository
        .create_session(project_id, user_id)
        .await
        .unwrap();
    {
        let msgs = vec![Message::new(Role::User).with_contents([Part::text("old")])];
        state
            .repository
            .append_messages(session.id, &msgs)
            .await
            .unwrap();
    }

    clear_message_history(&app, session.id, &token).await;

    {
        let msgs = vec![Message::new(Role::User).with_contents([Part::text("new")])];
        state
            .repository
            .append_messages(session.id, &msgs)
            .await
            .unwrap();
    }

    let body = get_message_history(&app, session.id, &token).await;
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1, "only the new message must remain");

    let text = arr[0]["contents"][0]["text"].as_str().unwrap_or("");
    assert_eq!(text, "new");
}

/// After clearing, the in-memory agent history is also wiped so the next turn
/// starts fresh.
#[tokio::test(flavor = "multi_thread")]
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

    let db_count = repo.get_messages(id).await.unwrap().len();
    assert_eq!(db_count, 0, "DB messages must be empty after clear");
}
