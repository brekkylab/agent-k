//! Integration tests for session metadata: title, last_message_at, unread tracking.

#[path = "common/mod.rs"]
mod common;

use ailoy::message::{Message, Part, Role};
use axum::http::StatusCode;
use uuid::Uuid;

// ── last_message_at ───────────────────────────────────────────────────────────

/// A freshly-created session has null last_message_at and null title, unread_count=0.
#[tokio::test]
async fn new_session_has_null_last_message_at_and_title() {
    let (app, _repo, _state) = common::make_app_repo_state().await;

    let username = format!("user_{}", Uuid::new_v4().simple());
    common::signup(&app, &username, "Password123!").await;
    let token = common::login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/projects/{project_id}/sessions"),
        &token,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create session failed: {body}");
    assert!(
        body["last_message_at"].is_null(),
        "last_message_at should be null: {body}"
    );
    assert!(body["title"].is_null(), "title should be null: {body}");
    assert_eq!(
        body["unread_count"].as_i64(),
        Some(0),
        "unread_count should be 0: {body}"
    );
}

/// last_message_at is set after append_messages, and returned by GET /sessions/{id}.
#[tokio::test]
async fn last_message_at_set_after_messages_appended() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice_lma", "Password123!").await;
    let alice_token = common::login(&app, "alice_lma", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    repo.append_messages(
        session.id,
        &common::to_new_msgs(
            &[
                Message::new(Role::User).with_contents([Part::text("hello")]),
                Message::new(Role::Assistant).with_contents([Part::text("hi")]),
            ],
            alice_id,
        ),
    )
    .await
    .unwrap();

    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "GET session failed: {body}");
    assert!(
        !body["last_message_at"].is_null(),
        "last_message_at should be set after appending messages: {body}"
    );
}

/// Updating share_mode does NOT change last_message_at.
#[tokio::test]
async fn share_mode_update_does_not_change_last_message_at() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice_smu", "Password123!").await;
    let alice_token = common::login(&app, "alice_smu", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    repo.append_messages(
        session.id,
        &common::to_new_msgs(
            &[Message::new(Role::User).with_contents([Part::text("hi")])],
            alice_id,
        ),
    )
    .await
    .unwrap();

    let (_, before) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &alice_token,
        None,
    )
    .await;
    let lma_before = before["last_message_at"].as_str().unwrap().to_string();

    common::update_share_mode(&app, &alice_token, session.id, "shared_readonly").await;

    let (_, after) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &alice_token,
        None,
    )
    .await;
    let lma_after = after["last_message_at"].as_str().unwrap().to_string();

    assert_eq!(
        lma_before, lma_after,
        "last_message_at must not change on share_mode update"
    );
}

// ── unread tracking ───────────────────────────────────────────────────────────

/// Creator's unread_count is 0 immediately after creating a session.
#[tokio::test]
async fn creator_has_zero_unread_on_new_session() {
    let (app, _repo, _state) = common::make_app_repo_state().await;

    let username = format!("user_{}", Uuid::new_v4().simple());
    common::signup(&app, &username, "Password123!").await;
    let token = common::login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap();

    let session_id = common::post_session_authed(&app, &token, project_id).await;

    let (_, body) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(
        body["unread_count"].as_i64(),
        Some(0),
        "new session: unread_count must be 0: {body}"
    );
}

/// After messages are appended directly to DB, non-sender sees unread_count > 0;
/// after GET messages, their unread_count resets to 0.
#[tokio::test]
async fn other_user_sees_unread_messages_until_they_read() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice_unread", "Password123!").await;
    let alice_token = common::login(&app, "alice_unread", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob_unread", "Password123!").await;
    let bob_token = common::login(&app, "bob_unread", "Password123!").await;
    common::add_member(&app, &alice_token, project_id_str, "bob_unread").await;

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    repo.update_session_share_mode(
        session.id,
        &agent_k_backend::repository::ShareMode::SharedChat,
    )
    .await
    .unwrap();

    // Append messages directly — simulates alice's side without going through send_message
    repo.append_messages(
        session.id,
        &common::to_new_msgs(
            &[
                Message::new(Role::User).with_contents([Part::text("Hey bob!")]),
                Message::new(Role::Assistant).with_contents([Part::text("hello")]),
            ],
            alice_id,
        ),
    )
    .await
    .unwrap();

    // Bob hasn't read anything yet → unread_count > 0
    let (_, body) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &bob_token,
        None,
    )
    .await;
    assert!(
        body["unread_count"].as_i64().unwrap_or(0) > 0,
        "bob should see unread messages before reading: {body}"
    );

    // Bob reads messages via GET /sessions/{id}/messages
    common::get_message_history(&app, session.id, &bob_token).await;

    // Now bob's unread_count should be 0
    let (_, body) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(
        body["unread_count"].as_i64(),
        Some(0),
        "bob's unread_count should be 0 after GET messages: {body}"
    );
}

/// list_sessions returns title, last_message_at, and unread_count per session.
#[tokio::test]
async fn list_sessions_includes_metadata() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice_list", "Password123!").await;
    let alice_token = common::login(&app, "alice_list", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    repo.append_messages(
        session.id,
        &common::to_new_msgs(
            &[Message::new(Role::User).with_contents([Part::text("test")])],
            alice_id,
        ),
    )
    .await
    .unwrap();
    repo.set_session_title(session.id, "Test session title")
        .await
        .unwrap();

    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/projects/{project_id_str}/sessions"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list sessions failed: {body}");
    let items = body["items"].as_array().unwrap();
    assert!(!items.is_empty(), "should have at least one session");
    let item = &items[0];
    assert_eq!(
        item["title"].as_str(),
        Some("Test session title"),
        "title should be set: {item}"
    );
    assert!(
        !item["last_message_at"].is_null(),
        "last_message_at should be set: {item}"
    );
    assert!(
        item["unread_count"].is_number(),
        "unread_count should be present: {item}"
    );
}

/// Forked session inherits the source title and starts with unread_count=0 for the forker.
#[tokio::test]
async fn fork_inherits_title_and_has_zero_unread() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice_fork", "Password123!").await;
    let alice_token = common::login(&app, "alice_fork", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let source = repo.create_session(project_id, alice_id).await.unwrap();
    repo.set_session_title(source.id, "Parent title")
        .await
        .unwrap();
    repo.append_messages(
        source.id,
        &common::to_new_msgs(
            &[
                Message::new(Role::User).with_contents([Part::text("hello")]),
                Message::new(Role::Assistant).with_contents([Part::text("world")]),
            ],
            alice_id,
        ),
    )
    .await
    .unwrap();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{}/fork", source.id),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "fork failed: {body}");
    assert_eq!(
        body["title"].as_str(),
        Some("Parent title"),
        "forked session should inherit parent title: {body}"
    );
    assert_eq!(
        body["unread_count"].as_i64(),
        Some(0),
        "forked session should have unread_count=0 for creator: {body}"
    );
    assert!(
        !body["last_message_at"].is_null(),
        "forked session should have last_message_at: {body}"
    );
}

/// After the first message stream, a `session_title_updated` WebSocket event must be
/// broadcast on `AppState::ws_tx` with the correct session_id, project_id, and a non-empty title.
#[tokio::test]
async fn first_message_stream_broadcasts_title_via_websocket() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, _repo, state) = common::make_app_repo_state().await;

    let username = format!("user_{}", Uuid::new_v4().simple());
    common::signup(&app, &username, "Password123!").await;
    let token = common::login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap().to_string();
    let session_id = common::post_session_authed(&app, &token, &project_id).await;

    // Subscribe before triggering the stream so no event is missed.
    let mut ws_rx = state.ws_tx.subscribe();

    common::send_message_stream_raw(&app, session_id, "What is the capital of France?", &token)
        .await;

    // Title generation is concurrent with (and outlasts) the SSE stream.
    // Wait up to 30 s for the broadcast event.
    let timeout = std::time::Duration::from_secs(30);
    loop {
        match tokio::time::timeout(timeout, ws_rx.recv()).await {
            Ok(Ok(agent_k_backend::events::WsEvent::SessionTitleUpdated {
                session_id: sid,
                project_id: pid,
                title,
            })) => {
                assert_eq!(sid, session_id.to_string(), "event session_id mismatch");
                assert_eq!(pid, project_id, "event project_id mismatch");
                assert!(!title.is_empty(), "broadcasted title must not be empty");
                assert!(
                    title.len() <= agent_k_backend::services::session_title::TITLE_MAX_LEN,
                    "title exceeds TITLE_MAX_LEN: {title:?}"
                );
                break;
            }
            Ok(Err(_)) => panic!("ws_tx channel closed before title event arrived"),
            Err(_) => panic!("no session_title_updated WS event received within {timeout:?}"),
        }
    }
}

/// send_message on the first message triggers fire-and-forget LLM title generation.
/// The title should be stored within a reasonable time after the response returns.
#[tokio::test]
async fn send_message_generates_title_via_llm() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, _repo, _state) = common::make_app_repo_state().await;

    let username = format!("user_{}", Uuid::new_v4().simple());
    common::signup(&app, &username, "Password123!").await;
    let token = common::login(&app, &username, "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap();

    let session_id = common::post_session_authed(&app, &token, project_id).await;

    // First message triggers fire-and-forget title generation
    common::send_message(&app, session_id, "What is the capital of France?", &token).await;

    // Poll until title is set (the spawn runs concurrently; give it up to 30s)
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let (_, body) = common::authed(
            &app,
            "GET",
            &format!("/sessions/{session_id}"),
            &token,
            None,
        )
        .await;

        if !body["title"].is_null() {
            let title = body["title"].as_str().unwrap();
            assert!(!title.is_empty(), "generated title should not be empty");
            assert!(
                title.len() <= 60,
                "title should be within 60-char cap: {title:?}"
            );
            break;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "title was not generated within 30 seconds"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

// ── repository-level unit tests ───────────────────────────────────────────────

/// mark_session_read and count_session_unread work correctly via the repo API.
#[tokio::test]
async fn repository_mark_and_count_unread() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    // Set up user + project + session via the HTTP API so we stay within public boundaries
    let alice_info = common::signup(&app, "alice_repo_unread", "Password123!").await;
    let alice_token = common::login(&app, "alice_repo_unread", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let bob_info = common::signup(&app, "bob_repo_unread", "Password123!").await;
    let bob_id = Uuid::parse_str(bob_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, alice_id).await.unwrap();

    // No messages → unread is 0 for everyone
    let unread = repo.count_session_unread(session.id, bob_id).await.unwrap();
    assert_eq!(unread, 0, "no messages → unread should be 0");

    // Append 2 messages
    repo.append_messages(
        session.id,
        &common::to_new_msgs(
            &[
                Message::new(Role::User).with_contents([Part::text("msg1")]),
                Message::new(Role::Assistant).with_contents([Part::text("msg2")]),
            ],
            alice_id,
        ),
    )
    .await
    .unwrap();

    // bob hasn't read anything → unread = 2
    let unread_bob = repo.count_session_unread(session.id, bob_id).await.unwrap();
    assert_eq!(unread_bob, 2, "bob should see 2 unread");

    // mark bob as read
    repo.mark_session_read(session.id, bob_id).await.unwrap();
    let unread_after = repo.count_session_unread(session.id, bob_id).await.unwrap();
    assert_eq!(unread_after, 0, "after mark_read, unread should be 0");

    // Append 1 more message
    repo.append_messages(
        session.id,
        &common::to_new_msgs(
            &[Message::new(Role::Assistant).with_contents([Part::text("new")])],
            alice_id,
        ),
    )
    .await
    .unwrap();
    let unread_new = repo.count_session_unread(session.id, bob_id).await.unwrap();
    assert_eq!(
        unread_new, 1,
        "after new message, bob should see 1 unread again"
    );
}

/// set_title saves the title and GET /sessions/{id} returns it.
#[tokio::test]
async fn set_title_persisted_in_response() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice_title", "Password123!").await;
    let alice_token = common::login(&app, "alice_title", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, alice_id).await.unwrap();

    // Initially null
    let (_, body) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &alice_token,
        None,
    )
    .await;
    assert!(
        body["title"].is_null(),
        "title should be null initially: {body}"
    );

    // Set title
    repo.set_session_title(session.id, "My session title")
        .await
        .unwrap();

    // Verify via GET
    let (_, body) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(
        body["title"].as_str(),
        Some("My session title"),
        "title should be set after set_title: {body}"
    );
}

/// After clear_message_history: last_message_at is null, unread_count is 0.
#[tokio::test]
async fn clear_message_history_resets_metadata() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice_clr", "Password123!").await;
    let alice_token = common::login(&app, "alice_clr", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = uuid::Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = uuid::Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, alice_id).await.unwrap();

    repo.append_messages(
        session.id,
        &common::to_new_msgs(
            &[
                Message::new(Role::User).with_contents([Part::text("hello")]),
                Message::new(Role::Assistant).with_contents([Part::text("hi")]),
            ],
            alice_id,
        ),
    )
    .await
    .unwrap();

    // Mark read so session_reads has a row
    repo.mark_session_read(session.id, alice_id).await.unwrap();

    // Pre-condition: last_message_at is set
    let (_, before) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &alice_token,
        None,
    )
    .await;
    assert!(
        !before["last_message_at"].is_null(),
        "last_message_at should be set before clear: {before}"
    );

    // Clear via HTTP API
    common::clear_message_history(&app, session.id, &alice_token).await;

    // Post-condition: last_message_at is null, unread_count is 0
    let (status, after) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "GET failed: {after}");
    assert!(
        after["last_message_at"].is_null(),
        "last_message_at should be null after clear: {after}"
    );
    assert_eq!(
        after["unread_count"].as_i64(),
        Some(0),
        "unread_count should be 0 after clear: {after}"
    );
}

/// set_title is a no-op if title is already set.
#[tokio::test]
async fn set_title_does_not_overwrite_existing() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice_title2", "Password123!").await;
    let alice_token = common::login(&app, "alice_title2", "Password123!").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    repo.set_session_title(session.id, "First title")
        .await
        .unwrap();
    repo.set_session_title(session.id, "Second title")
        .await
        .unwrap();

    let (_, body) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{}", session.id),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(
        body["title"].as_str(),
        Some("First title"),
        "title should not be overwritten: {body}"
    );
}
