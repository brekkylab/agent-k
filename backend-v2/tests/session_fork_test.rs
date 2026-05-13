#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use agent_k_backend::{repository::AppRepository, state::AppState};
use ailoy::{
    agent::{AgentBuilder, default_provider_mut},
    lang_model::LangModelProvider,
};
use axum::http::StatusCode;
use uuid::Uuid;

// Register a fake openai provider so AgentBuilder::build() can validate model names
// in tests that inject agents but never actually run them.
fn ensure_test_provider() {
    let mut provider = default_provider_mut();
    provider.models.insert(
        "openai/*".into(),
        LangModelProvider::openai("fake-key-for-test".into()),
    );
}

// ── helpers ───────────────────────────────────────────────────────────────────

async fn make_app_repo_state() -> (axum::Router, AppRepository, Arc<AppState>) {
    let repo = common::make_repo().await;
    let store = common::make_test_store();
    let state = Arc::new(AppState::new(
        repo.clone(),
        store,
        common::test_jwt_config(),
    ));
    let app = common::make_app_with_state(state.clone());
    (app, repo, state)
}

// ── fork_not_found ────────────────────────────────────────────────────────────

#[tokio::test]
async fn fork_not_found() {
    let (app, _repo, _state) = make_app_repo_state().await;

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

// ── fork_no_access ────────────────────────────────────────────────────────────

#[tokio::test]
async fn fork_no_access() {
    let (app, repo, _state) = make_app_repo_state().await;

    // alice owns a private session
    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();
    let session = repo.create_session(project_id, alice_id).await.unwrap();

    // charlie is not a member of alice's project
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

// ── fork_busy ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fork_busy() {
    let (app, repo, state) = make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    // Create session via repo — bypasses sandbox creation
    let session = repo.create_session(project_id, alice_id).await.unwrap();

    // Inject a minimal agent and hold its lock.
    // ensure_test_provider() registers a fake openai provider so build() succeeds;
    // the agent never runs so the fake key is never used.
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

    // Fork while the lock is held → 423 before any sandbox call
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

// ── fork_copies_messages ──────────────────────────────────────────────────────
//
// Verifies: 201 response, correct metadata, and message history is copied.
// Requires microsandbox runtime.

#[tokio::test]
#[ignore = "requires microsandbox runtime"]
async fn fork_copies_messages() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, repo, _state) = make_app_repo_state().await;

    let alice_info = common::signup(&app, "alice", "password123").await;
    let alice_token = common::login(&app, "alice", "password123").await;
    let alice_project = common::get_personal_project(&app, &alice_token).await;
    let project_id = Uuid::parse_str(alice_project["id"].as_str().unwrap()).unwrap();
    let alice_id = Uuid::parse_str(alice_info["id"].as_str().unwrap()).unwrap();

    // Seed a session with messages directly via repo (no sandbox needed for this step)
    let session_a = repo.create_session(project_id, alice_id).await.unwrap();
    let msgs = vec![
        ailoy::message::Message::new(ailoy::message::Role::User)
            .with_contents([ailoy::message::Part::text("hello")]),
        ailoy::message::Message::new(ailoy::message::Role::Assistant)
            .with_contents([ailoy::message::Part::text("hi there")]),
    ];
    repo.append_messages(session_a.id, &msgs).await.unwrap();

    // Fork via HTTP — this requires sandbox
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

    // Message history must be identical
    let source_msgs = repo.get_messages(session_a.id).await.unwrap();
    let fork_msgs = repo.get_messages(fork_id).await.unwrap();
    assert_eq!(
        fork_msgs.len(),
        source_msgs.len(),
        "fork must copy all messages from source"
    );
}

// ── fork_chat_member_can_fork ─────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires microsandbox runtime"]
async fn fork_chat_member_can_fork() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, repo, _state) = make_app_repo_state().await;

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

// ── fork_readonly_member_can_fork ─────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires microsandbox runtime"]
async fn fork_readonly_member_can_fork() {
    dotenvy::dotenv().ok();
    common::setup_provider().await;

    let (app, repo, _state) = make_app_repo_state().await;

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
