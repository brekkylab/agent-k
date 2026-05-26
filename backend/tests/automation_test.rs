#[path = "common/mod.rs"]
mod common;

use axum::http::StatusCode;
use serde_json::json;

async fn signup_and_personal_project(
    app: &axum::Router,
    username: &str,
) -> (String, String) {
    common::signup(app, username, "password123").await;
    let token = common::login(app, username, "password123").await;
    let project = common::get_personal_project(app, &token).await;
    let pid = project["id"].as_str().unwrap().to_string();
    (token, pid)
}

async fn create_automation(
    app: &axum::Router,
    token: &str,
    project_id: &str,
    name: &str,
    prompts: Vec<&str>,
) -> serde_json::Value {
    let (status, body) = common::authed(
        app,
        "POST",
        "/automations",
        token,
        Some(json!({
            "project_id": project_id,
            "name": name,
            "description": null,
            "prompts": prompts,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create automation: {body}");
    body
}

// ─── automation CRUD ─────────────────────────────────────────────────────────

#[tokio::test]
async fn automation_crud_happy_path() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;

    // create
    let created =
        create_automation(&app, &token, &pid, "daily report", vec!["step one", "step two"])
            .await;
    let auto_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["name"], "daily report");
    assert_eq!(created["prompts"].as_array().unwrap().len(), 2);

    // list
    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/automations?project_id={pid}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    // get
    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/automations/{auto_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], auto_id);

    // update — rename + replace prompts
    let (status, body) = common::authed(
        &app,
        "PATCH",
        &format!("/automations/{auto_id}"),
        &token,
        Some(json!({
            "name": "renamed",
            "prompts": ["just one"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "renamed");
    assert_eq!(body["prompts"], json!(["just one"]));

    // delete
    let (status, _) = common::authed(
        &app,
        "DELETE",
        &format!("/automations/{auto_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // gone → 404
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/automations/{auto_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn non_member_cannot_access_automation() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (alice_token, pid) = signup_and_personal_project(&app, "alice").await;
    let created = create_automation(&app, &alice_token, &pid, "secret", vec!["p"]).await;
    let auto_id = created["id"].as_str().unwrap();

    common::signup(&app, "bob", "password123").await;
    let bob_token = common::login(&app, "bob", "password123").await;

    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/automations/{auto_id}"),
        &bob_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_automation_rejects_empty_name() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;

    let (status, _) = common::authed(
        &app,
        "POST",
        "/automations",
        &token,
        Some(json!({ "project_id": pid, "name": "   ", "prompts": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ─── triggers ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn webhook_trigger_returns_plaintext_token_once() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "wh", vec!["p"]).await;
    let auto_id = auto["id"].as_str().unwrap().to_string();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/automations/{auto_id}/triggers"),
        &token,
        Some(json!({ "kind": "webhook" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create trigger: {body}");
    let plaintext = body["webhook_token"]
        .as_str()
        .expect("plaintext token returned on creation")
        .to_string();
    assert_eq!(plaintext.len(), 64);
    let trigger_id = body["trigger"]["id"].as_str().unwrap().to_string();

    // GET trigger should NOT include plaintext (it lives only in CreatedTriggerResponse)
    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/automations/{auto_id}/triggers/{trigger_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("webhook_token").is_none());
}

#[tokio::test]
async fn cron_trigger_no_token() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "cr", vec!["p"]).await;
    let auto_id = auto["id"].as_str().unwrap().to_string();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/automations/{auto_id}/triggers"),
        &token,
        Some(json!({ "kind": "cron", "expr": "*/5 * * * *", "tz": "UTC" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(body["webhook_token"].is_null());
    assert_eq!(body["trigger"]["kind"], "cron");
}

#[tokio::test]
async fn trigger_kind_is_immutable() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "imm", vec!["p"]).await;
    let auto_id = auto["id"].as_str().unwrap().to_string();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/automations/{auto_id}/triggers"),
        &token,
        Some(json!({ "kind": "cron", "expr": "* * * * *" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let tid = body["trigger"]["id"].as_str().unwrap().to_string();

    let (status, _) = common::authed(
        &app,
        "PATCH",
        &format!("/automations/{auto_id}/triggers/{tid}"),
        &token,
        Some(json!({
            "spec": { "kind": "webhook" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // toggling enabled is allowed
    let (status, body) = common::authed(
        &app,
        "PATCH",
        &format!("/automations/{auto_id}/triggers/{tid}"),
        &token,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], false);
}

#[tokio::test]
async fn list_triggers_and_delete() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "list", vec!["p"]).await;
    let auto_id = auto["id"].as_str().unwrap().to_string();

    // create two triggers
    let (_, _) = common::authed(
        &app,
        "POST",
        &format!("/automations/{auto_id}/triggers"),
        &token,
        Some(json!({ "kind": "cron", "expr": "* * * * *" })),
    )
    .await;
    let (_, t2) = common::authed(
        &app,
        "POST",
        &format!("/automations/{auto_id}/triggers"),
        &token,
        Some(json!({ "kind": "webhook" })),
    )
    .await;
    let t2_id = t2["trigger"]["id"].as_str().unwrap().to_string();

    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/automations/{auto_id}/triggers"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);

    let (status, _) = common::authed(
        &app,
        "DELETE",
        &format!("/automations/{auto_id}/triggers/{t2_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, body) = common::authed(
        &app,
        "GET",
        &format!("/automations/{auto_id}/triggers"),
        &token,
        None,
    )
    .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
}

// ─── runs / events ──────────────────────────────────────────────────────────

#[tokio::test]
async fn manual_run_queues_session_and_initial_events() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "manual", vec!["hi", "bye"]).await;
    let auto_id = auto["id"].as_str().unwrap().to_string();

    let (status, body) = common::authed(
        &app,
        "POST",
        &format!("/automations/{auto_id}/runs"),
        &token,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create run: {body}");
    assert_eq!(body["status"], "queued");
    assert!(body["session_id"].is_string(), "session_id present");
    assert!(body["trigger_id"].is_null(), "manual run has no trigger");
    let run_id = body["id"].as_str().unwrap().to_string();

    // events: triggered + queued recorded synchronously by the handler.
    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/automations/{auto_id}/runs/{run_id}/events"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let kinds: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"triggered"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"queued"), "kinds: {kinds:?}");
}

#[tokio::test]
async fn list_runs_empty() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "r", vec!["p"]).await;
    let auto_id = auto["id"].as_str().unwrap().to_string();

    let (status, body) = common::authed(
        &app,
        "GET",
        &format!("/automations/{auto_id}/runs"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_automations_no_filter_returns_user_scoped() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);

    let (alice_token, alice_pid) = signup_and_personal_project(&app, "alice").await;
    create_automation(&app, &alice_token, &alice_pid, "alice's auto", vec!["p"]).await;

    let (bob_token, bob_pid) = signup_and_personal_project(&app, "bob").await;
    create_automation(&app, &bob_token, &bob_pid, "bob's auto", vec!["p"]).await;
    create_automation(&app, &bob_token, &bob_pid, "bob's other", vec!["q"]).await;

    // No filter → alice sees only her own automations (not bob's).
    let (status, body) = common::authed(&app, "GET", "/automations", &alice_token, None).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "alice's auto");

    // No filter → bob sees only his two.
    let (status, body) = common::authed(&app, "GET", "/automations", &bob_token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);

    // Filtering by other user's project → 403.
    let (status, _) = common::authed(
        &app,
        "GET",
        &format!("/automations?project_id={bob_pid}"),
        &alice_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn missing_resources_return_404() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, _) = signup_and_personal_project(&app, "alice").await;

    let bogus = uuid::Uuid::new_v4();
    for path in [
        format!("/automations/{bogus}"),
        format!("/automations/{bogus}/triggers"),
        format!("/automations/{bogus}/triggers/{bogus}"),
        format!("/automations/{bogus}/runs"),
        format!("/automations/{bogus}/runs/{bogus}"),
        format!("/automations/{bogus}/runs/{bogus}/events"),
    ] {
        let (status, _) = common::authed(&app, "GET", &path, &token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "expected 404 for {path}");
    }
}

// ─── webhook firing (auth-exempt route) ──────────────────────────────────────

async fn fire_webhook(
    app: &axum::Router,
    bearer: Option<&str>,
    idempotency_key: Option<&str>,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let mut builder = Request::builder()
        .method("POST")
        .uri("/webhooks/automations")
        .header("content-type", "application/json");
    if let Some(t) = bearer {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    if let Some(k) = idempotency_key {
        builder = builder.header("idempotency-key", k);
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

async fn make_webhook_trigger(
    app: &axum::Router,
    token: &str,
    auto_id: &str,
) -> (String, String) {
    let (status, body) = common::authed(
        app,
        "POST",
        &format!("/automations/{auto_id}/triggers"),
        token,
        Some(json!({ "kind": "webhook" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create webhook trigger: {body}");
    let tid = body["trigger"]["id"].as_str().unwrap().to_string();
    let plaintext = body["webhook_token"].as_str().unwrap().to_string();
    (tid, plaintext)
}

#[tokio::test]
async fn webhook_fire_with_valid_token_queues_run() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "auto", vec!["hello"]).await;
    let aid = auto["id"].as_str().unwrap().to_string();
    let (_tid, plaintext) = make_webhook_trigger(&app, &token, &aid).await;

    let (status, body) =
        fire_webhook(&app, Some(&plaintext), None, r#"{"event":"ping"}"#).await;
    assert_eq!(status, StatusCode::ACCEPTED, "fire webhook: {body}");
    assert_eq!(body["status"], "queued");
    assert!(body["run_id"].is_string());
    assert!(body["session_id"].is_string());

    // Triggered event payload should encode the webhook source.
    let run_id = body["run_id"].as_str().unwrap();
    let (_, events) = common::authed(
        &app,
        "GET",
        &format!("/automations/{aid}/runs/{run_id}/events"),
        &token,
        None,
    )
    .await;
    let triggered = events["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "triggered")
        .expect("triggered event present");
    assert_eq!(triggered["payload"]["source"], "webhook");
}

#[tokio::test]
async fn webhook_fire_missing_bearer_returns_401() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "auto", vec!["x"]).await;
    let aid = auto["id"].as_str().unwrap().to_string();
    let (_tid, _plaintext) = make_webhook_trigger(&app, &token, &aid).await;

    let (status, _) = fire_webhook(&app, None, None, r#"{}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_fire_wrong_token_returns_401() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "auto", vec!["x"]).await;
    let aid = auto["id"].as_str().unwrap().to_string();
    let (_tid, _plaintext) = make_webhook_trigger(&app, &token, &aid).await;

    let (status, _) = fire_webhook(&app, Some("wrong-token-value"), None, r#"{}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_fire_disabled_trigger_returns_409() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "auto", vec!["x"]).await;
    let aid = auto["id"].as_str().unwrap().to_string();
    let (tid, plaintext) = make_webhook_trigger(&app, &token, &aid).await;

    // Disable the trigger.
    let (status, _) = common::authed(
        &app,
        "PATCH",
        &format!("/automations/{aid}/triggers/{tid}"),
        &token,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = fire_webhook(&app, Some(&plaintext), None, r#"{}"#).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn webhook_idempotency_key_returns_same_run_on_repeat() {
    let repo = common::make_repo().await;
    let app = common::make_app_with_repo(repo);
    let (token, pid) = signup_and_personal_project(&app, "alice").await;
    let auto = create_automation(&app, &token, &pid, "auto", vec!["x"]).await;
    let aid = auto["id"].as_str().unwrap().to_string();
    let (_tid, plaintext) = make_webhook_trigger(&app, &token, &aid).await;

    let key = "deploy-2026-05-19-abc";

    let (s1, b1) = fire_webhook(&app, Some(&plaintext), Some(key), r#"{}"#).await;
    assert_eq!(s1, StatusCode::ACCEPTED);
    let run1_id = b1["run_id"].as_str().unwrap().to_string();

    // Same key → same response (replay), still 202 Accepted with the
    // original run's identifiers.
    let (s2, b2) = fire_webhook(&app, Some(&plaintext), Some(key), r#"{}"#).await;
    assert_eq!(s2, StatusCode::ACCEPTED, "replay should reuse run: {b2}");
    assert_eq!(b2["run_id"].as_str().unwrap(), run1_id);

    // A different idempotency key → fresh run.
    let (s3, b3) = fire_webhook(&app, Some(&plaintext), Some("other-key"), r#"{}"#).await;
    assert_eq!(s3, StatusCode::ACCEPTED);
    assert_ne!(b3["run_id"].as_str().unwrap(), run1_id);

    // No header → always fires fresh (caller opted out of idempotency).
    let (s4, b4) = fire_webhook(&app, Some(&plaintext), None, r#"{}"#).await;
    assert_eq!(s4, StatusCode::ACCEPTED);
    assert_ne!(b4["run_id"].as_str().unwrap(), run1_id);
}
