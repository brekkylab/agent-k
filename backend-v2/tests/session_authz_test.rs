#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;
use axum::http::StatusCode;

// Helper: build an app and repo together so we can seed sessions via the repo.
async fn make_app_and_repo() -> (axum::Router, agent_k_backend::repository::AppRepository) {
    let repo = common::make_repo().await;
    let store = common::make_test_store();
    let state = Arc::new(agent_k_backend::state::AppState::new(
        repo.clone(),
        store,
        common::test_jwt_config(),
    ));
    let app = common::make_app_with_state(state);
    (app, repo)
}

// ── private_session_not_accessible_to_member ─────────────────────────────────

#[tokio::test]
async fn private_session_not_accessible_to_member() {
    let (app, repo) = make_app_and_repo().await;

    // alice signs up, bob signs up; bob joins alice's project
    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = uuid::Uuid::parse_str(project_id_str).unwrap();
    let alice_id = uuid::Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    // alice seeds a private session directly via repo
    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;

    // alice can access her own private session
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "alice should access her own private session");

    // bob (a member of the project) cannot access alice's private session → 404
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "bob should not see alice's private session");
}

// ── non_member_cannot_access_any_session ─────────────────────────────────────

#[tokio::test]
async fn non_member_cannot_access_any_session() {
    let (app, repo) = make_app_and_repo().await;

    // alice signs up; charlie is NOT a member of alice's project
    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = uuid::Uuid::parse_str(project_id_str).unwrap();
    let alice_id = uuid::Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "charlie", "password123").await;
    let charlie_token = common::login(&app, "charlie", "password123").await;

    // alice seeds a shared_chat session directly via repo, then changes share_mode via repo
    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;
    repo.update_session_share_mode(session_id, &agent_k_backend::repository::ShareMode::SharedChat)
        .await
        .unwrap();

    // charlie (not a member) tries to GET the session → 404
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &charlie_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "non-member should not access any session");
}

// ── shared_readonly_allows_read_but_not_send ──────────────────────────────────

#[tokio::test]
async fn shared_readonly_allows_read_but_not_send() {
    let (app, repo) = make_app_and_repo().await;

    // alice signs up; bob joins alice's project
    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = uuid::Uuid::parse_str(project_id_str).unwrap();
    let alice_id = uuid::Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    // alice creates a session (private) then promotes it to shared_readonly via HTTP
    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;
    common::update_share_mode(&app, &alice_token, session_id, "shared_readonly").await;

    // bob can GET messages → 200
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}/messages"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "bob should be able to read shared_readonly session");

    // bob cannot POST a message → 403
    let (status, _) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{session_id}/messages"),
        &bob_token,
        Some(serde_json::json!({ "content": "hello" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "bob should not be able to send to shared_readonly session");
}

// ── only_creator_can_change_share_mode ───────────────────────────────────────

#[tokio::test]
async fn only_creator_can_change_share_mode() {
    let (app, repo) = make_app_and_repo().await;

    // alice signs up; bob joins alice's project
    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = uuid::Uuid::parse_str(project_id_str).unwrap();
    let alice_id = uuid::Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    // alice creates a shared_chat session so bob can see it
    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;
    repo.update_session_share_mode(session_id, &agent_k_backend::repository::ShareMode::SharedChat)
        .await
        .unwrap();

    // bob (chat member) tries to PATCH the session's share_mode → 403
    let (status, body) = common::authed(
        &app,
        "PATCH",
        &format!("/sessions/{session_id}"),
        &bob_token,
        Some(serde_json::json!({ "share_mode": "private" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "non-creator should not change share mode: {body}");

    // alice (creator) can change it → 200
    let (status, body) = common::authed(
        &app,
        "PATCH",
        &format!("/sessions/{session_id}"),
        &alice_token,
        Some(serde_json::json!({ "share_mode": "private" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "creator should be able to change share mode: {body}");
}

// ── private_session_not_in_project_list ──────────────────────────────────────

#[tokio::test]
async fn private_session_not_in_project_list() {
    let (app, repo) = make_app_and_repo().await;

    // alice signs up; bob joins alice's project
    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = uuid::Uuid::parse_str(project_id_str).unwrap();
    let alice_id = uuid::Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    // alice creates a private session via repo
    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;

    // bob lists sessions → should be empty (private session not visible)
    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/projects/{project_id_str}/sessions"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list sessions failed: {body}");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 0, "bob should not see alice's private session: {body}");

    // alice changes session to shared_readonly via HTTP
    common::update_share_mode(&app, &alice_token, session_id, "shared_readonly").await;

    // bob lists sessions → now sees the session
    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/projects/{project_id_str}/sessions"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list sessions failed: {body}");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "bob should now see the shared_readonly session: {body}");
    assert_eq!(
        items[0]["id"].as_str().unwrap(),
        session_id.to_string(),
        "listed session id mismatch"
    );
}
