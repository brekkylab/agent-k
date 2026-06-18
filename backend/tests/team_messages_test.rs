//! Team messages: persisted user-to-user messages ('@' mentions) that never
//! reach the agent. Covers the two safety invariants (LLM-replay exclusion and
//! fork column preservation), unread/mention flags, and authz.

mod common;

use agent_k_backend::{
    events::WsEvent,
    handlers::build_replay_history,
    repository::DbMessageKind,
};
use ailoy::message::{Message, Part, Role};
use uuid::Uuid;

use common::{
    add_member, authed, get_personal_project, login, make_app_repo_state, post_session_authed,
    signup, to_new_msgs, update_share_mode,
};

async fn post_team_message(
    app: &axum::Router,
    token: &str,
    session_id: Uuid,
    content: &str,
    mentions: &[Uuid],
) -> (axum::http::StatusCode, serde_json::Value) {
    authed(
        app,
        "POST",
        &format!("/sessions/{session_id}/team-messages"),
        token,
        Some(serde_json::json!({
            "content": content,
            "mentions": mentions.iter().map(Uuid::to_string).collect::<Vec<_>>(),
        })),
    )
    .await
}

/// Owner + one chat member on the owner's personal project, with a shared_chat
/// session. Returns (app, repo, state, owner_token, member_token, member_id, session_id, project_id).
async fn shared_chat_fixture(
    prefix: &str,
) -> (
    axum::Router,
    agent_k_backend::repository::AppRepository,
    std::sync::Arc<agent_k_backend::state::AppState>,
    String,
    String,
    Uuid,
    Uuid,
    String,
) {
    let (app, repo, state) = make_app_repo_state().await;
    let owner = format!("{prefix}_owner");
    let member = format!("{prefix}_member");
    signup(&app, &owner, "Password123!").await;
    let member_info = signup(&app, &member, "Password123!").await;
    let member_id = Uuid::parse_str(member_info["id"].as_str().unwrap()).unwrap();
    let owner_token = login(&app, &owner, "Password123!").await;

    let project = get_personal_project(&app, &owner_token).await;
    let project_id = project["id"].as_str().unwrap().to_string();
    add_member(&app, &owner_token, &project_id, &member).await;
    let member_token = login(&app, &member, "Password123!").await;

    let session_id = post_session_authed(&app, &owner_token, &project_id).await;
    update_share_mode(&app, &owner_token, session_id, "shared_chat").await;

    (
        app,
        repo,
        state,
        owner_token,
        member_token,
        member_id,
        session_id,
        project_id,
    )
}

#[tokio::test]
async fn team_message_persists_and_broadcasts_without_run() {
    let (app, repo, state, owner_token, _mt, member_id, session_id, _pid) =
        shared_chat_fixture("tm_basic").await;

    let mut ws_rx = state.ws_tx.subscribe();

    let (status, body) =
        post_team_message(&app, &owner_token, session_id, "@member ping", &[member_id]).await;
    assert_eq!(status, axum::http::StatusCode::CREATED, "body: {body}");
    assert_eq!(body["message_kind"], "team");
    assert_eq!(body["mentions"][0], member_id.to_string());
    assert_eq!(body["sender"]["kind"], "user");

    // Exactly one broadcast, and it is TeamMessagePosted — no run lifecycle.
    match ws_rx.try_recv() {
        Ok(WsEvent::TeamMessagePosted {
            session_id: sid,
            project_id: pid,
            message,
        }) => {
            assert_eq!(sid, session_id.to_string());
            assert!(!pid.is_empty());
            assert_eq!(message.mentions, vec![member_id]);
        }
        other => panic!("expected TeamMessagePosted, got {other:?}"),
    }
    assert!(
        ws_rx.try_recv().is_err(),
        "no further events (AgentRunStarted must not fire)"
    );

    // Visible in the history API with kind + mentions.
    let (status, history) = authed(
        &app,
        "GET",
        &format!("/sessions/{session_id}/messages"),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let items = history["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["message_kind"], "team");
    assert_eq!(items[0]["mentions"][0], member_id.to_string());

    // Persisted row carries the team kind.
    let rows = repo.get_messages(session_id).await.unwrap();
    assert_eq!(rows[0].message_kind, DbMessageKind::Team);
    assert_eq!(rows[0].mentions, vec![member_id]);
}

#[tokio::test]
async fn replay_history_excludes_team_messages() {
    let (app, repo, _state, owner_token, _mt, member_id, session_id, _pid) =
        shared_chat_fixture("tm_replay").await;

    // One chat turn (seeded at the repo level — a real run needs a sandbox)…
    let owner_id = repo.get_session(session_id).await.unwrap().unwrap().creator_id;
    repo.append_messages(
        session_id,
        &to_new_msgs(
            &[
                Message::new(Role::User).with_contents([Part::text("real question")]),
                Message::new(Role::Assistant).with_contents([Part::text("real answer")]),
            ],
            owner_id,
        ),
    )
    .await
    .unwrap();

    // …then a team message in between future turns.
    let (status, _) =
        post_team_message(&app, &owner_token, session_id, "side chat", &[member_id]).await;
    assert_eq!(status, axum::http::StatusCode::CREATED);

    let rows = repo.get_messages(session_id).await.unwrap();
    assert_eq!(rows.len(), 3);

    // The single point where rows become agent history must drop the team row.
    let history = build_replay_history(rows);
    assert_eq!(history.len(), 2, "team message leaked into LLM history");
}

#[tokio::test]
async fn unread_mention_flags_across_sessions_and_mark_read() {
    let (app, _repo, _state, owner_token, member_token, member_id, s1, project_id) =
        shared_chat_fixture("tm_unread").await;

    // Second shared session in the same project — the batch query must keep
    // per-session flags straight (positional-bind order regression guard).
    let s2 = post_session_authed(&app, &owner_token, &project_id).await;
    update_share_mode(&app, &owner_token, s2, "shared_chat").await;

    let (st, _) = post_team_message(&app, &owner_token, s1, "@member look", &[member_id]).await;
    assert_eq!(st, axum::http::StatusCode::CREATED);
    let (st, _) = post_team_message(&app, &owner_token, s2, "no mention here", &[]).await;
    assert_eq!(st, axum::http::StatusCode::CREATED);

    let list = |token: String| {
        let app = app.clone();
        let project_id = project_id.clone();
        async move {
            let (status, body) = authed(
                &app,
                "GET",
                &format!("/sessions?project_ref={project_id}"),
                &token,
                None,
            )
            .await;
            assert_eq!(status, axum::http::StatusCode::OK);
            body["items"].as_array().unwrap().clone()
        }
    };

    let items = list(member_token.clone()).await;
    let find = |items: &[serde_json::Value], id: Uuid| {
        items
            .iter()
            .find(|s| s["id"] == id.to_string())
            .cloned()
            .unwrap()
    };
    let v1 = find(&items, s1);
    let v2 = find(&items, s2);
    assert_eq!(v1["unread_count"], 1);
    assert_eq!(v1["unread_mention"], true, "mentioned session must flag");
    assert_eq!(v2["unread_count"], 1);
    assert_eq!(v2["unread_mention"], false, "unmentioned session must not");

    // Reading the session (GET messages marks read) clears both.
    let (st, _) = authed(
        &app,
        "GET",
        &format!("/sessions/{s1}/messages"),
        &member_token,
        None,
    )
    .await;
    assert_eq!(st, axum::http::StatusCode::OK);

    let items = list(member_token.clone()).await;
    let v1 = find(&items, s1);
    assert_eq!(v1["unread_count"], 0);
    assert_eq!(v1["unread_mention"], false);
}

#[tokio::test]
async fn authz_and_validation() {
    let (app, _repo, _state, owner_token, member_token, member_id, session_id, project_id) =
        shared_chat_fixture("tm_authz").await;

    // Non-member: 404 (existence not revealed).
    signup(&app, "tm_authz_outsider", "Password123!").await;
    let outsider_token = login(&app, "tm_authz_outsider", "Password123!").await;
    let (st, _) = post_team_message(&app, &outsider_token, session_id, "hi", &[]).await;
    assert_eq!(st, axum::http::StatusCode::NOT_FOUND);

    // Mentioning a non-member: 400.
    let (st, body) = post_team_message(&app, &owner_token, session_id, "hi", &[Uuid::new_v4()]).await;
    assert_eq!(st, axum::http::StatusCode::BAD_REQUEST, "body: {body}");

    // Blank content: 400 (defense in depth; the UI trims before sending).
    let (st, body) = post_team_message(&app, &owner_token, session_id, "   ", &[]).await;
    assert_eq!(st, axum::http::StatusCode::BAD_REQUEST, "body: {body}");

    // Read-only share mode: member may read but not post.
    update_share_mode(&app, &owner_token, session_id, "shared_readonly").await;
    let (st, _) = post_team_message(&app, &member_token, session_id, "hi", &[]).await;
    assert_eq!(st, axum::http::StatusCode::FORBIDDEN);

    // Private session: team messages are rejected outright (silent-no-op guard).
    let private_session = post_session_authed(&app, &owner_token, &project_id).await;
    let (st, body) =
        post_team_message(&app, &owner_token, private_session, "hi", &[member_id]).await;
    assert_eq!(st, axum::http::StatusCode::BAD_REQUEST, "body: {body}");
}

#[tokio::test]
async fn fork_preserves_team_kind_and_replay_exclusion() {
    let (app, repo, _state, owner_token, _mt, member_id, session_id, _pid) =
        shared_chat_fixture("tm_fork").await;

    let owner_id = repo.get_session(session_id).await.unwrap().unwrap().creator_id;
    repo.append_messages(
        session_id,
        &to_new_msgs(
            &[Message::new(Role::User).with_contents([Part::text("chat row")])],
            owner_id,
        ),
    )
    .await
    .unwrap();
    let (st, _) =
        post_team_message(&app, &owner_token, session_id, "team row", &[member_id]).await;
    assert_eq!(st, axum::http::StatusCode::CREATED);

    // Repo-level fork: the column-copy SQL is what the invariant hangs on.
    // (The HTTP fork handler additionally clones the sandbox, which tests lack.)
    let forked = repo
        .fork_session(session_id, Uuid::new_v4(), owner_id)
        .await
        .unwrap();

    let rows = repo.get_messages(forked.id).await.unwrap();
    assert_eq!(rows.len(), 2);
    let team_row = rows
        .iter()
        .find(|r| r.message_kind == DbMessageKind::Team)
        .expect("fork dropped message_kind — team rows degraded to chat");
    assert_eq!(team_row.mentions, vec![member_id]);

    // And the fork's cold replay still excludes it.
    let history = build_replay_history(rows);
    assert_eq!(history.len(), 1);
}
