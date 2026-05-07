#[path = "common/mod.rs"]
mod common;

use axum::http::StatusCode;

// ── signup_creates_personal_project ──────────────────────────────────────────

#[tokio::test]
async fn signup_creates_personal_project() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    common::signup(&app, "alice", "password123").await;
    let token = common::login(&app, "alice", "password123").await;

    let (status, body) = common::authed(&app, "GET", "/projects", &token, None).await;
    assert_eq!(status, StatusCode::OK, "list projects failed: {body}");

    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "expected exactly 1 project, got: {}", items.len());
    assert_eq!(items[0]["name"], "Personal");
}

// ── non_member_cannot_access_project ─────────────────────────────────────────

#[tokio::test]
async fn non_member_cannot_access_project() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    // alice signs up and gets her personal project
    common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = alice_project["id"].as_str().unwrap();

    // bob signs up
    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;

    // bob tries to access alice's project
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/projects/{project_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ── owner_can_invite_and_remove_member ────────────────────────────────────────

#[tokio::test]
async fn owner_can_invite_and_remove_member() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    // alice and bob sign up
    common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = alice_project["id"].as_str().unwrap();
    let alice_id = alice_project["owner_id"].as_str().unwrap();

    let bob_info = common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    let bob_id = bob_info["id"].as_str().unwrap();

    // alice invites bob → 204
    common::add_member(&app, &alice_token, project_id, "bob").await;

    // bob can now access project → 200
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/projects/{project_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "bob should be able to access project");

    // bob tries to remove alice → 403 (non-owner cannot remove other members)
    let (status, body) = common::authed(
        &app,
        "DELETE",
        &format!("/projects/{project_id}/members/{alice_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "bob should not be able to remove alice: {body}");

    // alice removes bob → 204
    let (status, body) = common::authed(
        &app,
        "DELETE",
        &format!("/projects/{project_id}/members/{bob_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "alice should be able to remove bob: {body}");

    // bob can no longer access project → 403
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/projects/{project_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "bob should be forbidden after removal");
}

// ── member_cannot_invite ──────────────────────────────────────────────────────

#[tokio::test]
async fn member_cannot_invite() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    // alice, bob, charlie sign up
    common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = alice_project["id"].as_str().unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;

    common::signup(&app, "charlie", "password123").await;

    // alice invites bob
    common::add_member(&app, &alice_token, project_id, "bob").await;

    // bob tries to invite charlie → 403
    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/projects/{project_id}/members"),
        &bob_token,
        Some(serde_json::json!({ "username": "charlie" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "member should not be able to invite: {body}");
}

// ── project_delete_cascades_sessions ─────────────────────────────────────────

#[tokio::test]
async fn project_delete_cascades_sessions() {
    use std::sync::Arc;

    // Build app with direct repo access so we can seed a session without
    // an AI provider (the HTTP create-session handler tries to build an agent).
    let repo = common::make_repo().await;
    let store = common::make_test_store();
    let state = Arc::new(agent_k_backend::state::AppState::new(
        repo.clone(),
        store,
        common::test_jwt_config(),
    ));
    let app = common::make_app_with_state(state);

    // alice signs up — personal project is auto-created
    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = uuid::Uuid::parse_str(project_id_str).unwrap();
    let alice_id = uuid::Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    // seed a session directly via the repository (no agent required)
    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;

    // verify the session is accessible
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "session should exist before project deletion");

    // alice deletes the project → 204
    let (status, body) = common::authed(
        &app,
        "DELETE",
        &format!("/projects/{project_id_str}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete project failed: {body}");

    // session should now return 404
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "session should be gone after project deletion");
}

// ── owner_leave_is_blocked ────────────────────────────────────────────────────

#[tokio::test]
async fn owner_leave_is_blocked() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    // alice signs up
    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_id = alice_info["id"].as_str().unwrap();
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = alice_project["id"].as_str().unwrap();

    // alice tries to remove herself from her own project → 400
    let (status, body) = common::authed(
        &app,
        "DELETE",
        &format!("/projects/{project_id}/members/{alice_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "owner leave should be blocked: {body}");
}
