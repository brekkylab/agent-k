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
        Some(json!({ "kind": "webhook", "dedupe": "payload_hash" })),
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
            "spec": { "kind": "webhook", "dedupe": null }
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
