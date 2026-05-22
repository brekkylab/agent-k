#[path = "common/mod.rs"]
mod common;

use ailoy::{
    agent::{AgentBuilder, default_provider_mut},
    lang_model::LangModelProvider,
};
use axum::http::StatusCode;
use uuid::Uuid;

fn ensure_test_provider() {
    let mut provider = default_provider_mut();
    provider.models.insert(
        "openai/*".into(),
        LangModelProvider::openai("fake-key-for-test".into()),
    );
}

// ── authz: private_session_not_accessible_to_member ──────────────────────────

#[tokio::test]
async fn private_session_not_accessible_to_member() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;

    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "alice should access her own private session"
    );

    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "bob should not see alice's private session"
    );
}

// ── authz: non_member_cannot_access_any_session ───────────────────────────────

#[tokio::test]
async fn non_member_cannot_access_any_session() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "charlie", "password123").await;
    let charlie_token = common::login(&app, "charlie", "password123").await;

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;
    repo.update_session_share_mode(
        session_id,
        &agent_k_backend::repository::ShareMode::SharedChat,
    )
    .await
    .unwrap();

    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &charlie_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "non-member should not access any session"
    );
}

// ── authz: shared_readonly_allows_read_but_not_send ──────────────────────────

#[tokio::test]
async fn shared_readonly_allows_read_but_not_send() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;
    common::update_share_mode(&app, &alice_token, session_id, "shared_readonly").await;

    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}/messages"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bob should be able to read shared_readonly session"
    );

    let (status, _) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{session_id}/messages"),
        &bob_token,
        Some(serde_json::json!({ "content": "hello" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bob should not be able to send to shared_readonly session"
    );
}

// ── authz: only_creator_can_change_share_mode ─────────────────────────────────

#[tokio::test]
async fn only_creator_can_change_share_mode() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;
    repo.update_session_share_mode(
        session_id,
        &agent_k_backend::repository::ShareMode::SharedChat,
    )
    .await
    .unwrap();

    let (status, body) = common::authed(
        &app,
        "PATCH",
        &format!("/sessions/{session_id}"),
        &bob_token,
        Some(serde_json::json!({ "share_mode": "private" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "non-creator should not change share mode: {body}"
    );

    let (status, body) = common::authed(
        &app,
        "PATCH",
        &format!("/sessions/{session_id}"),
        &alice_token,
        Some(serde_json::json!({ "share_mode": "private" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "creator should be able to change share mode: {body}"
    );
}

// ── authz: owner_can_access_member_private_session ────────────────────────────

#[tokio::test]
async fn owner_can_access_member_private_session() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_token = {
        common::signup(&app, "alice", "password123").await;
        common::login(&app, "alice", "password123").await
    };
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();

    let bob_info = common::signup(&app, "bob", "password123").await;
    let bob_id = Uuid::parse_str(bob_info["id"].as_str().unwrap()).unwrap();
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    let session = repo.create_session(project_id, bob_id).await.unwrap();
    let session_id = session.id;

    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "owner must be able to access any session including private ones"
    );

    let (status, _) = common::authed(
        &app,
        "DELETE",
        &format!("/sessions/{session_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "owner must be able to delete any session"
    );
}

// ── authz: removed_member_loses_session_access ────────────────────────────────

#[tokio::test]
async fn removed_member_loses_session_access() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_token = {
        common::signup(&app, "alice", "password123").await;
        common::login(&app, "alice", "password123").await
    };
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();

    let bob_info = common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    let bob_id = Uuid::parse_str(bob_info["id"].as_str().unwrap()).unwrap();
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    let session = repo.create_session(project_id, bob_id).await.unwrap();
    let session_id = session.id;

    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bob should access his session while still a member"
    );

    let (status, _) = common::authed(
        &app,
        "DELETE",
        &format!("/projects/{project_id_str}/members/{bob_id}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "remove member failed");

    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "removed member must not access sessions in the project"
    );
}

// ── authz: owner_sees_all_sessions_in_project_list ───────────────────────────

#[tokio::test]
async fn owner_sees_all_sessions_in_project_list() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let bob_info = common::signup(&app, "bob", "password123").await;
    let bob_id = Uuid::parse_str(bob_info["id"].as_str().unwrap()).unwrap();
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    repo.create_session(project_id, alice_id).await.unwrap();
    repo.create_session(project_id, bob_id).await.unwrap();

    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/projects/{project_id_str}/sessions"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "list failed: {body}");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(
        items.len(),
        2,
        "owner must see all sessions including private ones from members, got: {body}"
    );
}

// ── authz: private_session_not_in_project_list ───────────────────────────────

#[tokio::test]
async fn private_session_not_in_project_list() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    let session_id = session.id;

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
    assert_eq!(
        items.len(),
        0,
        "bob should not see alice's private session: {body}"
    );

    common::update_share_mode(&app, &alice_token, session_id, "shared_readonly").await;

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
    assert_eq!(
        items.len(),
        1,
        "bob should now see the shared_readonly session: {body}"
    );
    assert_eq!(
        items[0]["id"].as_str().unwrap(),
        session_id.to_string(),
        "listed session id mismatch"
    );
}

// ── fork: fork_not_found ──────────────────────────────────────────────────────

#[tokio::test]
async fn fork_not_found() {
    let (app, _repo, _state) = common::make_app_repo_state().await;

    common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;

    let fake_id = Uuid::new_v4();
    let (status, _) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{fake_id}/fork"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "fork of nonexistent session should be 404"
    );
}

// ── fork: fork_no_access ──────────────────────────────────────────────────────

#[tokio::test]
async fn fork_no_access() {
    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();
    let session = repo.create_session(project_id, alice_id).await.unwrap();

    common::signup(&app, "charlie", "password123").await;
    let charlie_token = common::login(&app, "charlie", "password123").await;

    let (status, _) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{}/fork", session.id),
        &charlie_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "non-member fork should return 404"
    );
}

// ── fork: fork_busy ───────────────────────────────────────────────────────────

#[tokio::test]
async fn fork_busy() {
    let (app, repo, state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let session = repo.create_session(project_id, alice_id).await.unwrap();

    ensure_test_provider();
    let agent = AgentBuilder::new("openai/gpt-4o-mini")
        .build()
        .expect("AgentBuilder::build() must succeed with a registered provider");
    state.insert_agent(session.id, agent);
    let _guard = state
        .get_agent(&session.id)
        .expect("agent just inserted must be present")
        .try_lock_owned()
        .expect("freshly inserted agent must not be locked");

    let (status, _) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{}/fork", session.id),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::LOCKED,
        "fork while agent is busy should return 423"
    );
}

// ── fork: fork_copies_messages ────────────────────────────────────────────────

#[tokio::test]
async fn fork_copies_messages() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    let session_a = repo.create_session(project_id, alice_id).await.unwrap();
    let msgs = vec![
        ailoy::message::Message::new(ailoy::message::Role::User)
            .with_contents([ailoy::message::Part::text("hello")]),
        ailoy::message::Message::new(ailoy::message::Role::Assistant)
            .with_contents([ailoy::message::Part::text("hi there")]),
    ];
    repo.append_messages(session_a.id, &common::to_new_msgs(&msgs, alice_id))
        .await
        .unwrap();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{}/fork", session_a.id),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "fork should return 201: {body}"
    );

    let fork_id = Uuid::parse_str(body["id"].as_str().unwrap()).unwrap();
    assert_ne!(fork_id, session_a.id, "fork must get a new id");
    assert_eq!(
        body["share_mode"].as_str().unwrap(),
        "private",
        "fork must be private"
    );
    assert_eq!(
        body["creator_id"].as_str().unwrap(),
        alice_id.to_string(),
        "fork creator must be the requesting user"
    );
    assert_eq!(
        body["project_id"].as_str().unwrap(),
        project_id.to_string(),
        "fork must belong to the same project"
    );

    let source_msgs = repo.get_messages(session_a.id).await.unwrap();
    let fork_msgs = repo.get_messages(fork_id).await.unwrap();
    assert_eq!(
        fork_msgs.len(),
        source_msgs.len(),
        "fork must copy all messages from source"
    );
}

// ── fork: fork_chat_member_can_fork ──────────────────────────────────────────

#[tokio::test]
async fn fork_chat_member_can_fork() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    repo.update_session_share_mode(
        session.id,
        &agent_k_backend::repository::ShareMode::SharedChat,
    )
    .await
    .unwrap();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{}/fork", session.id),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "chat member must be able to fork: {body}"
    );
}

// ── fork: fork_readonly_member_can_fork ──────────────────────────────────────

#[tokio::test]
async fn fork_readonly_member_can_fork() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, repo, _state) = common::make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id_str = alice_project["id"].as_str().unwrap();
    let project_id = Uuid::parse_str(project_id_str).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;
    common::add_member(&app, &alice_token, project_id_str, "bob").await;

    let session = repo.create_session(project_id, alice_id).await.unwrap();
    repo.update_session_share_mode(
        session.id,
        &agent_k_backend::repository::ShareMode::SharedReadonly,
    )
    .await
    .unwrap();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/sessions/{}/fork", session.id),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "readonly member must be able to fork: {body}"
    );
}

// ── session CRUD ──────────────────────────────────────────────────────────────

/// DELETE /sessions/{id} returns 404 for an unknown session.
#[tokio::test]
async fn delete_unknown_session_returns_404() {
    let (app, _repo, _state) = common::make_app_repo_state().await;

    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    common::signup(&app, &username, "Password123!").await;
    let token = common::login(&app, &username, "Password123!").await;

    let result = common::try_delete_session(&app, Uuid::new_v4(), &token).await;
    assert!(result.is_err(), "DELETE on unknown session must return 404");
}
