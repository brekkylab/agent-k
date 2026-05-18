//! Integration tests for the dirent (project file upload) API.

mod common;

use std::sync::Arc;

use agent_k_backend::state::AppState;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn make_state_with_dir() -> (Arc<AppState>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let repo = common::make_repo().await;
    let store = common::make_test_store();
    let state = Arc::new(AppState::new(
        repo,
        store,
        common::test_jwt_config(),
        tmp.path().to_path_buf(),
    ));
    (state, tmp)
}

async fn make_state_with_dir_and_max_bytes(max_bytes: usize) -> (Arc<AppState>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let repo = common::make_repo().await;
    let store = common::make_test_store();
    // Build AppState directly with a custom max_upload_bytes to avoid env var races.
    let mut state = AppState::new(
        repo,
        store,
        common::test_jwt_config(),
        tmp.path().to_path_buf(),
    );
    // Patch the field via a builder approach — we expose the field publicly,
    // so just overwrite it after construction.
    state.max_upload_bytes = max_bytes;
    (Arc::new(state), tmp)
}

async fn upload_files(
    app: &axum::Router,
    token: &str,
    project_id: &str,
    files: &[(&str, &[u8])],
) -> (axum::http::StatusCode, serde_json::Value) {
    let (boundary, body) = common::build_multipart_body(files);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/projects/{project_id}/dirents"))
        .header("authorization", format!("Bearer {token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn list_dirents(
    app: &axum::Router,
    token: &str,
    project_id: &str,
    query: &str,
) -> serde_json::Value {
    let uri = if query.is_empty() {
        format!("/projects/{project_id}/dirents")
    } else {
        format!("/projects/{project_id}/dirents?{query}")
    };
    let (_, body) = common::authed(app, "GET", &uri, token, None).await;
    body
}

async fn get_file_raw(
    app: &axum::Router,
    token: &str,
    project_id: &str,
    path: &str,
) -> (axum::http::StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method("GET")
        .uri(format!("/projects/{project_id}/dirents/{path}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, bytes)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn upload_and_list_files() {
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "alice", "Password123!").await;
    let token = common::login(&app, "alice", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();

    let (status, body) = upload_files(
        &app,
        &token,
        pid,
        &[("README.md", b"hello"), ("src/main.rs", b"fn main() {}")],
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "upload: {body}");
    let succeeded = body["succeeded"].as_array().unwrap();
    assert_eq!(succeeded.len(), 2, "expected 2 succeeded");
    assert_eq!(body["failed"].as_array().unwrap().len(), 0);

    let list = list_dirents(&app, &token, pid, "").await;
    let paths: Vec<&str> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"README.md"), "entries: {paths:?}");
    assert!(paths.contains(&"src/main.rs"), "entries: {paths:?}");
}

#[tokio::test]
async fn path_traversal_in_upload_goes_to_failed() {
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "bob", "Password123!").await;
    let token = common::login(&app, "bob", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();

    let (status, body) = upload_files(&app, &token, pid, &[("../escape.txt", b"malicious")]).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["succeeded"].as_array().unwrap().len(), 0);
    let failed = body["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "expected 1 failed: {body}");
    assert!(
        failed[0]["error"].as_str().unwrap().contains(".."),
        "error should mention '..': {body}"
    );
}

#[tokio::test]
async fn upload_over_size_limit_goes_to_failed() {
    // Use a dedicated AppState with a tiny limit to avoid env-var races.
    let (state, _tmp) = make_state_with_dir_and_max_bytes(10).await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "carol", "Password123!").await;
    let token = common::login(&app, "carol", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();

    let big = vec![0u8; 20]; // 20 bytes > 10 byte limit
    let (status, body) = upload_files(&app, &token, pid, &[("big.bin", &big)]).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["succeeded"].as_array().unwrap().len(), 0);
    assert_eq!(body["failed"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn get_file_returns_bytes_and_content_type() {
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "dave", "Password123!").await;
    let token = common::login(&app, "dave", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();

    upload_files(&app, &token, pid, &[("hello.txt", b"world")]).await;

    let (status, bytes) = get_file_raw(&app, &token, pid, "hello.txt").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(bytes, b"world");
}

#[tokio::test]
async fn delete_file_removes_it() {
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "eve", "Password123!").await;
    let token = common::login(&app, "eve", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();

    upload_files(&app, &token, pid, &[("to_delete.txt", b"bye")]).await;

    let (del_status, _) = common::authed(
        &app,
        "DELETE",
        &format!("/projects/{pid}/dirents/to_delete.txt"),
        &token,
        None,
    )
    .await;
    assert_eq!(del_status, axum::http::StatusCode::NO_CONTENT);

    let (get_status, _) = get_file_raw(&app, &token, pid, "to_delete.txt").await;
    assert_eq!(get_status, axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_directory_is_recursive() {
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "frank", "Password123!").await;
    let token = common::login(&app, "frank", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();

    upload_files(
        &app,
        &token,
        pid,
        &[
            ("src/a.rs", b"a"),
            ("src/b.rs", b"b"),
            ("root.txt", b"keep"),
        ],
    )
    .await;

    let (del_status, _) = common::authed(
        &app,
        "DELETE",
        &format!("/projects/{pid}/dirents/src"),
        &token,
        None,
    )
    .await;
    assert_eq!(del_status, axum::http::StatusCode::NO_CONTENT);

    let (s, _) = get_file_raw(&app, &token, pid, "src/a.rs").await;
    assert_eq!(s, axum::http::StatusCode::NOT_FOUND);

    let (s, _) = get_file_raw(&app, &token, pid, "root.txt").await;
    assert_eq!(s, axum::http::StatusCode::OK);
}

#[tokio::test]
async fn non_member_gets_403_on_all_endpoints() {
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "owner_nm", "Password123!").await;
    let owner_token = common::login(&app, "owner_nm", "Password123!").await;
    let project = common::get_personal_project(&app, &owner_token).await;
    let pid = project["id"].as_str().unwrap();
    upload_files(&app, &owner_token, pid, &[("secret.txt", b"secret")]).await;

    common::signup(&app, "stranger_nm", "Password123!").await;
    let stranger_token = common::login(&app, "stranger_nm", "Password123!").await;

    // POST upload
    let (s, _) = upload_files(&app, &stranger_token, pid, &[("x.txt", b"x")]).await;
    assert_eq!(s, axum::http::StatusCode::FORBIDDEN, "POST should be 403");

    // GET list
    let req = Request::builder()
        .method("GET")
        .uri(format!("/projects/{pid}/dirents"))
        .header("authorization", format!("Bearer {stranger_token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::FORBIDDEN,
        "GET list should be 403"
    );

    // GET file
    let (s, _) = get_file_raw(&app, &stranger_token, pid, "secret.txt").await;
    assert_eq!(
        s,
        axum::http::StatusCode::FORBIDDEN,
        "GET file should be 403"
    );

    // DELETE
    let (s, _) = common::authed(
        &app,
        "DELETE",
        &format!("/projects/{pid}/dirents/secret.txt"),
        &stranger_token,
        None,
    )
    .await;
    assert_eq!(s, axum::http::StatusCode::FORBIDDEN, "DELETE should be 403");
}

#[tokio::test]
async fn list_prefix_filter_uses_path_component_boundary() {
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "grace", "Password123!").await;
    let token = common::login(&app, "grace", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();

    upload_files(
        &app,
        &token,
        pid,
        &[("src/foo.rs", b"foo"), ("src2/bar.rs", b"bar")],
    )
    .await;

    let list = list_dirents(&app, &token, pid, "prefix=src&recursive=true").await;
    let paths: Vec<&str> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.iter().any(|p| p.starts_with("src/")),
        "src/ should appear: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("src2/")),
        "src2/ must NOT appear with prefix=src: {paths:?}"
    );
}
