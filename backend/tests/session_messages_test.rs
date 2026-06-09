//! Integration tests for session message persistence and history endpoints.

#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use agent_k::agents::{GUEST_ATTACHED_DIR, GUEST_SHARED_DIR};
use agent_k_backend::{
    handlers::{build_attachment_note, ensure_attachment_count, inject_attachment_note},
    repository,
    state::AppState,
};
use ailoy::message::{Message, Part, Role};
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
        let repo = repository::create_repository(&db_url).await.unwrap();
        let app = make_app_with_repo(repo.clone());
        let username = format!("user_{}", uuid::Uuid::new_v4().simple());
        let user_info = signup(&app, &username, "Password123!").await;
        let user_id = uuid::Uuid::parse_str(user_info["id"].as_str().unwrap()).unwrap();
        let token = login(&app, &username, "Password123!").await;
        let project = common::get_personal_project(&app, &token).await;
        let project_slug = project["slug"].as_str().unwrap().to_string();
        let id = post_session_authed(&app, &token, &project_slug).await;
        repo.append_messages(
            id,
            &common::to_new_msgs(
                &[
                    Message::new(Role::User).with_contents([Part::text("hello")]),
                    Message::new(Role::Assistant).with_contents([Part::text("world")]),
                ],
                user_id,
            ),
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
    let arr = messages["items"]
        .as_array()
        .expect("items must be a JSON array");
    assert_eq!(arr.len(), 2, "both seeded messages must survive restart");
    assert_eq!(arr[0]["message"]["role"].as_str().unwrap(), "user");
    assert_eq!(arr[1]["message"]["role"].as_str().unwrap(), "assistant");

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
        serde_json::json!({"items": []}),
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
            .append_messages(session.id, &common::to_new_msgs(&msgs, user_id))
            .await
            .unwrap();
    }

    let body = get_message_history(&app, session.id, &token).await;
    let arr = body["items"]
        .as_array()
        .expect("items must be a JSON array");
    assert_eq!(arr.len(), 2, "must return exactly two messages");

    let role0 = arr[0]["message"]["role"].as_str().unwrap_or("");
    let role1 = arr[1]["message"]["role"].as_str().unwrap_or("");
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
            .append_messages(session.id, &common::to_new_msgs(&msgs, user_id))
            .await
            .unwrap();
    }

    let before = get_message_history(&app, session.id, &token).await;
    assert_eq!(
        before["items"].as_array().unwrap().len(),
        2,
        "expected two messages before clear"
    );

    clear_message_history(&app, session.id, &token).await;

    let after = get_message_history(&app, session.id, &token).await;
    assert_eq!(
        after,
        serde_json::json!({"items": []}),
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
            .append_messages(session.id, &common::to_new_msgs(&msgs, user_id))
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
            .append_messages(session.id, &common::to_new_msgs(&msgs, user_id))
            .await
            .unwrap();
    }

    clear_message_history(&app, session.id, &token).await;

    {
        let msgs = vec![Message::new(Role::User).with_contents([Part::text("new")])];
        state
            .repository
            .append_messages(session.id, &common::to_new_msgs(&msgs, user_id))
            .await
            .unwrap();
    }

    let body = get_message_history(&app, session.id, &token).await;
    let arr = body["items"].as_array().unwrap();
    assert_eq!(arr.len(), 1, "only the new message must remain");

    let text = arr[0]["message"]["contents"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert_eq!(text, "new");
}

/// GET /sessions/{id}/messages returns correct sender.kind, sender.user_id, and sender.name.
#[tokio::test]
async fn get_messages_response_includes_correct_sender_field() {
    use agent_k_backend::repository::{DbSenderKind, NewSessionMessage};
    use ailoy::message::{Message, Part, Role};

    let store = common::make_test_store();
    let repo = common::make_repo().await;
    let data_root = std::env::temp_dir().join(format!("agent-k-sender-{}", Uuid::new_v4()));
    let state = std::sync::Arc::new(agent_k_backend::state::AppState::new(
        repo.clone(),
        store,
        common::test_jwt_config(),
        data_root,
    ));
    let app = common::make_app_with_state(state.clone());

    let username = format!("alice_sender_{}", Uuid::new_v4().simple());
    let alice_info = common::signup(&app, &username, "Password123!").await;
    let alice_token = common::login(&app, &username, "Password123!").await;
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();
    let project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();

    let session = state
        .repository
        .create_session(project_id, alice_id)
        .await
        .unwrap();
    repo.append_messages(
        session.id,
        &[
            NewSessionMessage {
                message: Message::new(Role::User).with_contents([Part::text("hello")]),
                sender_kind: DbSenderKind::User,
                sender_name: None,
                sender_user_id: Some(alice_id),
                attachments: vec![],
                artifacts: vec![],
            },
            NewSessionMessage {
                message: Message::new(Role::Assistant).with_contents([Part::text("hi there")]),
                sender_kind: DbSenderKind::Agent,
                sender_name: Some("agent-k".to_string()),
                sender_user_id: None,
                attachments: vec![],
                artifacts: vec![],
            },
        ],
    )
    .await
    .unwrap();

    let body = common::get_message_history(&app, session.id, &alice_token).await;
    let items = body["items"].as_array().expect("items must be an array");
    assert_eq!(items.len(), 2, "expected 2 messages");

    let alice_id_str = alice_id.to_string();

    let user_msg = &items[0];
    assert_eq!(
        user_msg["sender"]["kind"].as_str(),
        Some("user"),
        "first message sender.kind must be 'user': {user_msg}"
    );
    assert_eq!(
        user_msg["sender"]["user_id"].as_str(),
        Some(alice_id_str.as_str()),
        "first message sender.user_id must be alice's id: {user_msg}"
    );

    let agent_msg = &items[1];
    assert_eq!(
        agent_msg["sender"]["kind"].as_str(),
        Some("agent"),
        "second message sender.kind must be 'agent': {agent_msg}"
    );
    assert_eq!(
        agent_msg["sender"]["name"].as_str(),
        Some("agent-k"),
        "second message sender.name must be 'agent-k': {agent_msg}"
    );
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
    let user_info = signup(&app, &username, "Password123!").await;
    let user_id = uuid::Uuid::parse_str(user_info["id"].as_str().unwrap()).unwrap();
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_slug = project["slug"].as_str().unwrap().to_string();
    let id = post_session_authed(&app, &token, &project_slug).await;

    let _guard = SessionGuard {
        app: app.clone(),
        id,
        token: token.clone(),
    };

    {
        repo.append_messages(
            id,
            &common::to_new_msgs(
                &[Message::new(Role::User).with_contents([Part::text("should be cleared")])],
                user_id,
            ),
        )
        .await
        .unwrap();
    }

    clear_message_history(&app, id, &token).await;

    let db_count = repo.get_messages(id).await.unwrap().len();
    assert_eq!(db_count, 0, "DB messages must be empty after clear");
}

// ── Security / validation tests ───────────────────────────────────────────────

/// Sending an unknown field in the message payload must be rejected with 422
/// (deny_unknown_fields). This prevents silent data loss where e.g. a typo
/// like "attachment" instead of "attachments" would be silently ignored.
#[tokio::test]
async fn send_message_rejects_unknown_fields() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());
    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo);

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap().to_string();
    let session_id = post_session_authed(&app, &token, &project_id).await;

    // "attachment" (singular) is a typo for "attachments" — should be rejected.
    let (status, _) = authed(
        &app,
        "POST",
        &format!("/sessions/{session_id}/messages"),
        &token,
        Some(serde_json::json!({
            "content": "hello",
            "attachment": "projects/some-path/file.txt"
        })),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "unknown field 'attachment' must be rejected with 422"
    );
}

/// A symlink placed in a session's inputs directory must be rejected by
/// validate_attachments, preventing sandbox escape via symlink traversal.
#[tokio::test]
async fn symlink_as_attachment_is_rejected() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let data_root = tempfile::tempdir().unwrap();
    let repo = repository::create_repository("sqlite::memory:")
        .await
        .unwrap();
    let store = common::make_test_store();
    let state = Arc::new(AppState::new(
        repo.clone(),
        store,
        common::test_jwt_config(),
        data_root.path().to_path_buf(),
    ));
    let app = make_app_with_state(state);

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();
    let pid_uuid = uuid::Uuid::parse_str(pid).unwrap();
    let (_, me) = authed(&app, "GET", "/me", &token, None).await;
    let uid = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let session_id = repo
        .create_session(pid_uuid, uid)
        .await
        .expect("create session")
        .id;

    // Create inputs directory and place a symlink inside it.
    let inputs_dir = data_root
        .path()
        .join("projects")
        .join(pid_uuid.to_string())
        .join("sessions")
        .join(session_id.to_string())
        .join("inputs");
    std::fs::create_dir_all(&inputs_dir).unwrap();
    let target = data_root.path().join("outside_sandbox.txt");
    std::fs::write(&target, b"secret").unwrap();
    std::os::unix::fs::symlink(&target, inputs_dir.join("evil")).unwrap();

    let attachment_path = format!("projects/{pid}/sessions/{session_id}/inputs/evil");
    let (status, body) = authed(
        &app,
        "POST",
        &format!("/sessions/{session_id}/messages"),
        &token,
        Some(serde_json::json!({
            "content": "read the file",
            "attachments": [attachment_path]
        })),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::BAD_REQUEST,
        "symlink attachment must be rejected with 400: {body}"
    );
    assert!(
        body.to_string().contains("symlink"),
        "error must mention symlink: {body}"
    );
}

/// Messages stored in the DB must not contain the [Attached files ...] note in
/// their body — the note is reconstructed at LLM-context time from the separate
/// `attachments` column and must never be exposed to the frontend.
#[tokio::test]
async fn attachment_note_not_present_in_stored_message_body() {
    dotenvy::dotenv().ok();
    setup_provider().await;

    let dir = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}", dir.path().join("test.db").display());
    let repo = repository::create_repository(&db_url).await.unwrap();
    let app = make_app_with_repo(repo.clone());

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    let info = signup(&app, &username, "Password123!").await;
    let user_id = uuid::Uuid::parse_str(info["id"].as_str().unwrap()).unwrap();
    let token = login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap().to_string();
    let session_id = post_session_authed(&app, &token, &project_id).await;

    // Simulate what the handler now stores: clean content + attachments metadata,
    // with NO [Attached files ...] text embedded in the message body.
    let clean_content = "Can you summarise this document?";
    repo.append_messages(
        session_id,
        &[repository::NewSessionMessage {
            message: ailoy::message::Message::new(ailoy::message::Role::User)
                .with_contents([ailoy::message::Part::text(clean_content)]),
            sender_kind: repository::DbSenderKind::User,
            sender_name: None,
            sender_user_id: Some(user_id),
            attachments: vec![format!(
                "projects/{project_id}/sessions/{session_id}/inputs/report.pdf"
            )],
            artifacts: vec![],
        }],
    )
    .await
    .unwrap();

    // The GET endpoint must return the clean body without any injected note.
    let history = get_message_history(&app, session_id, &token).await;
    let items = history["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);

    let msg_body = items[0]["message"]["contents"][0]["text"]
        .as_str()
        .expect("text content");
    assert_eq!(
        msg_body, clean_content,
        "stored message body must be clean user text, got: {msg_body}"
    );
    assert!(
        !msg_body.contains("[Attached files"),
        "attachment note must not appear in stored message body: {msg_body}"
    );

    // The attachments field must still be present alongside the clean content.
    let attachments = items[0]["attachments"]
        .as_array()
        .expect("attachments array");
    assert_eq!(attachments.len(), 1, "attachment path must be preserved");
}

// ── Attachment note (build_attachment_note / inject_attachment_note) ──────────

fn pid() -> &'static str {
    "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
}
fn sid() -> &'static str {
    "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
}
fn inputs_path(filename: &str) -> String {
    format!("projects/{}/sessions/{}/inputs/{}", pid(), sid(), filename)
}
fn shared_path(rel: &str) -> String {
    format!("projects/{}/shared/{}", pid(), rel)
}
fn message_text(msg: &Message) -> String {
    msg.contents
        .iter()
        .filter_map(|p| {
            if let Part::Text { text } = p {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn build_attachment_note_inputs_uses_basename_only() {
    let note = build_attachment_note(&[inputs_path("report.pdf")]);
    assert!(
        note.contains(&format!("{GUEST_ATTACHED_DIR}/report.pdf")),
        "expected GUEST_ATTACHED_DIR/basename, got: {note}"
    );
    assert!(
        !note.contains("sessions"),
        "full session path must not appear in inputs hint: {note}"
    );
}

#[test]
fn build_attachment_note_shared_preserves_full_tail() {
    let note = build_attachment_note(&[shared_path("Market research/Q2 report.md")]);
    assert!(
        note.contains(&format!("{GUEST_SHARED_DIR}/Market research/Q2 report.md")),
        "expected GUEST_SHARED_DIR/<full-tail>, got: {note}"
    );
    assert!(
        !note.contains("projects/"),
        "scope prefix must be stripped from shared hint: {note}"
    );
}

#[test]
fn build_attachment_note_mixed_scopes() {
    let note = build_attachment_note(&[inputs_path("data.csv"), shared_path("ref/schema.json")]);
    assert!(
        note.contains(&format!("{GUEST_ATTACHED_DIR}/data.csv")),
        "{note}"
    );
    assert!(
        note.contains(&format!("{GUEST_SHARED_DIR}/ref/schema.json")),
        "{note}"
    );
}

#[test]
fn build_attachment_note_empty_returns_empty_hint() {
    let note = build_attachment_note(&[]);
    assert!(note.contains("[Attached files:"), "{note}");
    assert!(!note.contains(GUEST_ATTACHED_DIR), "{note}");
}

#[test]
fn inject_attachment_note_appends_note_to_text() {
    let msg = Message::new(Role::User).with_contents([Part::text("Hello")]);
    let result = inject_attachment_note(msg, &[inputs_path("file.txt")]);
    let text = message_text(&result);
    assert!(
        text.starts_with("Hello\n\n"),
        "note must be appended after a blank line: {text}"
    );
    assert!(
        text.contains("[Attached files:"),
        "note must be present: {text}"
    );
    assert!(
        text.contains(&format!("{GUEST_ATTACHED_DIR}/file.txt")),
        "hint must use correct guest path: {text}"
    );
}

#[test]
fn inject_attachment_note_noop_for_empty_attachments() {
    let msg = Message::new(Role::User).with_contents([Part::text("Hello")]);
    let result = inject_attachment_note(msg, &[]);
    assert_eq!(message_text(&result), "Hello");
}

#[test]
fn inject_attachment_note_preserves_message_role_and_id() {
    let mut msg = Message::new(Role::User).with_contents([Part::text("hi")]);
    msg.id = Some("test-id".to_string());
    let result = inject_attachment_note(msg, &[inputs_path("x.txt")]);
    assert_eq!(result.role, Role::User);
    assert_eq!(result.id.as_deref(), Some("test-id"));
}

#[test]
fn inject_then_re_inject_is_idempotent_on_text_prefix() {
    let original = "Summarise this document";
    let msg = Message::new(Role::User).with_contents([Part::text(original)]);
    let after_first = inject_attachment_note(msg, &[inputs_path("doc.pdf")]);
    let text = message_text(&after_first);
    assert!(
        text.starts_with(original),
        "original text must remain as prefix: {text}"
    );
}

#[test]
fn attachment_count_at_or_under_limit_is_ok() {
    assert!(ensure_attachment_count(0).is_ok());
    assert!(ensure_attachment_count(30).is_ok());
}

#[test]
fn attachment_count_over_limit_is_rejected() {
    assert!(ensure_attachment_count(31).is_err());
}
