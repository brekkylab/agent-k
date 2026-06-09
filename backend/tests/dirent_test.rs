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
    let state = Arc::new(AppState::new(
        repo,
        common::test_jwt_config(),
        tmp.path().to_path_buf(),
    ));
    (state, tmp)
}

async fn make_state_with_dir_and_max_bytes(max_bytes: usize) -> (Arc<AppState>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let repo = common::make_repo().await;
    // Build AppState directly with a custom max_upload_bytes to avoid env var races.
    let mut state = AppState::new(
        repo,
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
        .uri(format!("/dirents?path=projects/{project_id}/shared"))
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
        format!("/dirents?path=projects/{project_id}/shared")
    } else {
        format!("/dirents?path=projects/{project_id}/shared&{query}")
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
        .uri(format!("/dirents/projects/{project_id}/shared/{path}"))
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
    assert!(
        paths.contains(&format!("projects/{pid}/shared/README.md").as_str()),
        "entries: {paths:?}"
    );
    assert!(
        paths.contains(&format!("projects/{pid}/shared/src/main.rs").as_str()),
        "entries: {paths:?}"
    );
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
        &format!("/dirents/projects/{pid}/shared/to_delete.txt"),
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
        &format!("/dirents/projects/{pid}/shared/src"),
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
        .uri(format!("/dirents?path=projects/{pid}/shared"))
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
        &format!("/dirents/projects/{pid}/shared/secret.txt"),
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
    let scope_prefix = format!("projects/{pid}/shared");
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with(&format!("{scope_prefix}/src/"))),
        "src/ should appear: {paths:?}"
    );
    assert!(
        !paths
            .iter()
            .any(|p| p.starts_with(&format!("{scope_prefix}/src2/"))),
        "src2/ must NOT appear with prefix=src: {paths:?}"
    );
}

// ── PATCH /dirents (batch move/copy) ──────────────────────────────────────────

#[tokio::test]
async fn batch_op_rename_via_move_with_new_name() {
    // Rename = a single-source move whose destination is the same parent
    // and whose new_name carries the new filename. The unified move handler
    // must accept this and produce the new dirent in the response.
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);
    common::signup(&app, "grace", "Password123!").await;
    let token = common::login(&app, "grace", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();
    upload_files(&app, &token, pid, &[("a.txt", b"hi")]).await;

    let body = serde_json::json!({
        "op": "move",
        "sources": [format!("projects/{pid}/shared/a.txt")],
        "destination": format!("projects/{pid}/shared"),
        "new_name": "b.txt",
    });
    let (status, resp) = common::authed(&app, "PATCH", "/dirents", &token, Some(body)).await;
    assert_eq!(status, axum::http::StatusCode::OK, "rename body: {resp}");
    let succeeded = resp["succeeded"].as_array().unwrap();
    assert_eq!(succeeded.len(), 1);
    assert_eq!(succeeded[0]["path"], format!("projects/{pid}/shared/b.txt"));
    assert_eq!(resp["failed"].as_array().unwrap().len(), 0);

    let (gone_status, _) = get_file_raw(&app, &token, pid, "a.txt").await;
    assert_eq!(gone_status, axum::http::StatusCode::NOT_FOUND);
    let (here_status, here_bytes) = get_file_raw(&app, &token, pid, "b.txt").await;
    assert_eq!(here_status, axum::http::StatusCode::OK);
    assert_eq!(here_bytes, b"hi");
}

#[tokio::test]
async fn batch_op_copy_applies_finder_style_suffix() {
    // Copying into a folder that already contains the same name must NOT fail.
    // It should auto-append " copy" (and " copy 2", " copy 3"…) Finder-style.
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);
    common::signup(&app, "henry", "Password123!").await;
    let token = common::login(&app, "henry", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();
    upload_files(&app, &token, pid, &[("note.txt", b"x")]).await;

    let copy = |target: &str| {
        let target = target.to_string();
        let app = app.clone();
        let token = token.clone();
        let pid = pid.to_string();
        async move {
            let body = serde_json::json!({
                "op": "copy",
                "sources": [format!("projects/{pid}/shared/{target}")],
                "destination": format!("projects/{pid}/shared"),
            });
            common::authed(&app, "PATCH", "/dirents", &token, Some(body)).await
        }
    };

    let (s1, r1) = copy("note.txt").await;
    assert_eq!(s1, axum::http::StatusCode::OK, "{r1}");
    assert_eq!(
        r1["succeeded"][0]["path"],
        format!("projects/{pid}/shared/note copy.txt")
    );

    let (s2, r2) = copy("note.txt").await;
    assert_eq!(s2, axum::http::StatusCode::OK, "{r2}");
    assert_eq!(
        r2["succeeded"][0]["path"],
        format!("projects/{pid}/shared/note copy 2.txt")
    );
}

#[tokio::test]
async fn batch_op_move_folder_into_descendant_fails() {
    // Moving a folder into one of its own descendants would create an
    // unreachable cycle on disk; the handler must reject this with a clear
    // error in `failed` (not silently 500 or actually perform the rename).
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);
    common::signup(&app, "iris", "Password123!").await;
    let token = common::login(&app, "iris", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();
    upload_files(&app, &token, pid, &[("Outer/inner/keep.txt", b"y")]).await;

    let body = serde_json::json!({
        "op": "move",
        "sources": [format!("projects/{pid}/shared/Outer")],
        "destination": format!("projects/{pid}/shared/Outer/inner"),
    });
    let (status, resp) = common::authed(&app, "PATCH", "/dirents", &token, Some(body)).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{resp}");
    assert_eq!(resp["succeeded"].as_array().unwrap().len(), 0);
    let failed = resp["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "{resp}");
    let err = failed[0]["error"].as_str().unwrap();
    assert!(
        err.contains("itself") || err.contains("descendant"),
        "expected self-into-itself error, got: {err}"
    );
}

#[tokio::test]
async fn batch_op_partial_success_on_name_conflict() {
    // When some sources can be moved but others would collide, the response
    // must split into `succeeded` and `failed` rather than aborting the batch
    // (matches the upload partial-success contract).
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);
    common::signup(&app, "jay", "Password123!").await;
    let token = common::login(&app, "jay", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();
    upload_files(
        &app,
        &token,
        pid,
        &[
            ("dst/blocker.txt", b"x"),
            ("blocker.txt", b"y"),
            ("free.txt", b"z"),
        ],
    )
    .await;

    // Move both blocker.txt (will conflict with dst/blocker.txt) and free.txt (clean).
    let body = serde_json::json!({
        "op": "move",
        "sources": [
            format!("projects/{pid}/shared/blocker.txt"),
            format!("projects/{pid}/shared/free.txt"),
        ],
        "destination": format!("projects/{pid}/shared/dst"),
    });
    let (status, resp) = common::authed(&app, "PATCH", "/dirents", &token, Some(body)).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{resp}");
    let succeeded = resp["succeeded"].as_array().unwrap();
    let failed = resp["failed"].as_array().unwrap();
    assert_eq!(succeeded.len(), 1, "succeeded should hold free.txt: {resp}");
    assert_eq!(
        succeeded[0]["path"],
        format!("projects/{pid}/shared/dst/free.txt")
    );
    assert_eq!(failed.len(), 1, "failed should hold blocker.txt: {resp}");
    assert_eq!(
        failed[0]["path"],
        format!("projects/{pid}/shared/blocker.txt")
    );
    assert!(
        failed[0]["error"]
            .as_str()
            .unwrap()
            .contains("already exists"),
        "expected 'already exists', got: {}",
        failed[0]["error"]
    );
}

#[tokio::test]
async fn batch_op_all_failed_does_not_materialise_dest_dir() {
    // Regression: a batch op whose sources all fail must NOT leave an empty
    // destination directory on disk. Creation of dest_dir is deferred to the
    // first successful per-source op so that aborted batches leave no trace.
    let (state, tmp) = make_state_with_dir().await;
    let data_root = tmp.path().to_path_buf();
    let app = common::make_app_with_state(state);
    common::signup(&app, "jay", "Password123!").await;
    let token = common::login(&app, "jay", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();

    // Move two non-existent sources into a destination path that has never
    // been created before. Both sources fail at load_source.
    let body = serde_json::json!({
        "op": "move",
        "sources": [
            format!("projects/{pid}/shared/ghost-a.txt"),
            format!("projects/{pid}/shared/ghost-b.txt"),
        ],
        "destination": format!("projects/{pid}/shared/phantom-dir"),
    });
    let (status, resp) = common::authed(&app, "PATCH", "/dirents", &token, Some(body)).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{resp}");
    assert!(resp["succeeded"].as_array().unwrap().is_empty(), "{resp}");
    assert_eq!(resp["failed"].as_array().unwrap().len(), 2, "{resp}");

    let dest_on_disk = data_root
        .join("projects")
        .join(pid)
        .join("shared")
        .join("phantom-dir");
    assert!(
        !dest_on_disk.exists(),
        "destination dir must not be created when every source fails: {}",
        dest_on_disk.display()
    );
}

// ── Cross-scope copy tests ─────────────────────────────────────────────────────

/// Create a session directly via the repository (bypasses the session handler
/// which requires a live AI provider) and return its id.
async fn create_session_direct(
    repo: &agent_k_backend::repository::AppRepository,
    project_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> uuid::Uuid {
    repo.create_session(project_id, user_id)
        .await
        .expect("create_session failed")
        .id
}

#[tokio::test]
async fn cross_scope_copy_artifacts_to_shared_succeeds() {
    // Create a file directly in a session's artifacts directory on disk, then
    // copy it to the project's shared scope via the API.
    let (app, repo, state) = common::make_app_repo_state().await;
    let data_root = state.data_root.clone();

    common::signup(&app, "xsc_alice", "Password123!").await;
    let token = common::login(&app, "xsc_alice", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();
    let pid_uuid = uuid::Uuid::parse_str(pid).unwrap();

    // Get the user id from the token (decode via /me endpoint)
    let (_, me) = common::authed(&app, "GET", "/me", &token, None).await;
    let uid = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    // Create a session directly in the repo so we have a valid session id
    let sid = create_session_direct(&repo, pid_uuid, uid).await;

    // Write a file directly into the artifacts directory on disk
    let artifacts_dir = data_root
        .join("projects")
        .join(pid)
        .join("sessions")
        .join(sid.to_string())
        .join("artifacts");
    tokio::fs::create_dir_all(&artifacts_dir).await.unwrap();
    tokio::fs::write(artifacts_dir.join("test.txt"), b"artifact content")
        .await
        .unwrap();

    // Copy from artifacts scope to shared scope via PATCH /dirents
    let body = serde_json::json!({
        "op": "copy",
        "sources": [format!("projects/{pid}/sessions/{sid}/artifacts/test.txt")],
        "destination": format!("projects/{pid}/shared"),
    });
    let (status, resp) = common::authed(&app, "PATCH", "/dirents", &token, Some(body)).await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "cross-scope copy: {resp}"
    );

    let succeeded = resp["succeeded"].as_array().unwrap();
    assert_eq!(succeeded.len(), 1, "expected 1 succeeded: {resp}");
    assert_eq!(
        succeeded[0]["path"],
        format!("projects/{pid}/shared/test.txt"),
        "path in response: {resp}"
    );
    assert_eq!(resp["failed"].as_array().unwrap().len(), 0, "{resp}");

    // Verify the file actually exists on disk under shared/
    let shared_file = data_root
        .join("projects")
        .join(pid)
        .join("shared")
        .join("test.txt");
    assert!(
        shared_file.exists(),
        "file must exist in shared/ after cross-scope copy: {}",
        shared_file.display()
    );

    // Original should still exist (copy, not move)
    let src_file = artifacts_dir.join("test.txt");
    assert!(
        src_file.exists(),
        "source file must still exist after copy: {}",
        src_file.display()
    );
}

#[tokio::test]
async fn cross_scope_copy_across_projects_rejected() {
    // Copying from one project's artifacts to another project's shared must be
    // rejected with 404 (to hide cross-project existence).
    let (app, repo, state) = common::make_app_repo_state().await;
    let data_root = state.data_root.clone();

    // Set up user1 with project1
    common::signup(&app, "xsc_bob", "Password123!").await;
    let token1 = common::login(&app, "xsc_bob", "Password123!").await;
    let project1 = common::get_personal_project(&app, &token1).await;
    let pid1 = project1["id"].as_str().unwrap();
    let pid1_uuid = uuid::Uuid::parse_str(pid1).unwrap();
    let (_, me1) = common::authed(&app, "GET", "/me", &token1, None).await;
    let uid1 = uuid::Uuid::parse_str(me1["id"].as_str().unwrap()).unwrap();

    // Set up user2 with project2
    common::signup(&app, "xsc_carol", "Password123!").await;
    let token2 = common::login(&app, "xsc_carol", "Password123!").await;
    let project2 = common::get_personal_project(&app, &token2).await;
    let pid2 = project2["id"].as_str().unwrap();

    // Create a session in project1 and write a file to its artifacts
    let sid1 = create_session_direct(&repo, pid1_uuid, uid1).await;
    let artifacts_dir = data_root
        .join("projects")
        .join(pid1)
        .join("sessions")
        .join(sid1.to_string())
        .join("artifacts");
    tokio::fs::create_dir_all(&artifacts_dir).await.unwrap();
    tokio::fs::write(artifacts_dir.join("secret.txt"), b"secret")
        .await
        .unwrap();

    // User 2 tries to copy from project1's artifacts to project2's shared
    let body = serde_json::json!({
        "op": "copy",
        "sources": [format!("projects/{pid1}/sessions/{sid1}/artifacts/secret.txt")],
        "destination": format!("projects/{pid2}/shared"),
    });
    let (status, resp) = common::authed(&app, "PATCH", "/dirents", &token2, Some(body)).await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "cross-project copy must be rejected with 404: {resp}"
    );
}

#[tokio::test]
async fn cross_scope_move_rejected() {
    // Moving from one scope to another (even within the same project) must be
    // rejected with 400.
    let (app, repo, state) = common::make_app_repo_state().await;
    let data_root = state.data_root.clone();

    common::signup(&app, "xsc_dave", "Password123!").await;
    let token = common::login(&app, "xsc_dave", "Password123!").await;
    let project = common::get_personal_project(&app, &token).await;
    let pid = project["id"].as_str().unwrap();
    let pid_uuid = uuid::Uuid::parse_str(pid).unwrap();
    let (_, me) = common::authed(&app, "GET", "/me", &token, None).await;
    let uid = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    // Create a session and write a file to its artifacts
    let sid = create_session_direct(&repo, pid_uuid, uid).await;
    let artifacts_dir = data_root
        .join("projects")
        .join(pid)
        .join("sessions")
        .join(sid.to_string())
        .join("artifacts");
    tokio::fs::create_dir_all(&artifacts_dir).await.unwrap();
    tokio::fs::write(artifacts_dir.join("result.txt"), b"result")
        .await
        .unwrap();

    // Attempt to MOVE (not copy) from artifacts to shared — must be rejected
    let body = serde_json::json!({
        "op": "move",
        "sources": [format!("projects/{pid}/sessions/{sid}/artifacts/result.txt")],
        "destination": format!("projects/{pid}/shared"),
    });
    let (status, resp) = common::authed(&app, "PATCH", "/dirents", &token, Some(body)).await;
    assert_eq!(
        status,
        axum::http::StatusCode::BAD_REQUEST,
        "cross-scope move must be rejected with 400: {resp}"
    );
}

// ── Access-control tests ──────────────────────────────────────────────────────

/// A project member with ChatMember access to a shared session must NOT be able
/// to upload files to that session's inputs scope (only Admins/creators may).
#[tokio::test]
async fn chat_member_cannot_upload_to_session_inputs() {
    let (app, repo, state) = common::make_app_repo_state().await;
    let data_root = state.data_root.clone();

    // Alice: session creator
    common::signup(&app, "cm_alice", "Password123!").await;
    let alice_token = common::login(&app, "cm_alice", "Password123!").await;
    let project = common::get_personal_project(&app, &alice_token).await;
    let pid = project["id"].as_str().unwrap();
    let pid_uuid = uuid::Uuid::parse_str(pid).unwrap();
    let (_, me) = common::authed(&app, "GET", "/me", &alice_token, None).await;
    let alice_uid = uuid::Uuid::parse_str(me["id"].as_str().unwrap()).unwrap();

    let sid = create_session_direct(&repo, pid_uuid, alice_uid).await;

    // Share the session so Bob can join as ChatMember.
    common::update_share_mode(&app, &alice_token, sid, "shared_chat").await;

    // Bob: a project member who joins as ChatMember.
    common::signup(&app, "cm_bob", "Password123!").await;
    let bob_token = common::login(&app, "cm_bob", "Password123!").await;
    common::add_member(&app, &alice_token, pid, "cm_bob").await;

    // Ensure the session inputs directory exists.
    let inputs_dir = data_root
        .join("projects")
        .join(pid_uuid.to_string())
        .join("sessions")
        .join(sid.to_string())
        .join("inputs");
    std::fs::create_dir_all(&inputs_dir).unwrap();

    // Bob tries to upload to Alice's session inputs — must be 403.
    let (boundary, body) = common::build_multipart_body(&[("inject.txt", b"evil content")]);
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!(
            "/dirents?path=projects/{pid}/sessions/{sid}/inputs"
        ))
        .header("authorization", format!("Bearer {bob_token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(axum::body::Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::FORBIDDEN,
        "ChatMember must not be allowed to upload to session inputs"
    );

    // Alice (Admin) can still upload to her own inputs.
    let (boundary, body) = common::build_multipart_body(&[("file.txt", b"legit")]);
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!(
            "/dirents?path=projects/{pid}/sessions/{sid}/inputs"
        ))
        .header("authorization", format!("Bearer {alice_token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(axum::body::Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::OK,
        "Admin (session creator) must be able to upload to inputs"
    );
}

/// A non-member must receive 403 on the PATCH /dirents (batch copy/move) endpoint.
#[tokio::test]
async fn non_member_gets_403_on_patch_endpoint() {
    let (state, _tmp) = make_state_with_dir().await;
    let app = common::make_app_with_state(state);

    common::signup(&app, "nm_patch_owner", "Password123!").await;
    let owner_token = common::login(&app, "nm_patch_owner", "Password123!").await;
    let project = common::get_personal_project(&app, &owner_token).await;
    let pid = project["id"].as_str().unwrap();

    // Seed a file to use as a copy source.
    upload_files(&app, &owner_token, pid, &[("seed.txt", b"data")]).await;

    common::signup(&app, "nm_patch_stranger", "Password123!").await;
    let stranger_token = common::login(&app, "nm_patch_stranger", "Password123!").await;

    let body = serde_json::json!({
        "op": "copy",
        "sources": [format!("projects/{pid}/shared/seed.txt")],
        "destination": format!("projects/{pid}/shared/copy"),
    });
    let (status, resp) =
        common::authed(&app, "PATCH", "/dirents", &stranger_token, Some(body)).await;
    assert_eq!(
        status,
        axum::http::StatusCode::FORBIDDEN,
        "PATCH /dirents must return 403 for non-members: {resp}"
    );
}
