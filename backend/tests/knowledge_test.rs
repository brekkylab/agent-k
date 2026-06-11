#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use agent_k_backend::state::AppState;
use axum::http::StatusCode;
use common::{
    authed, build_multipart_body, get_personal_project, login, make_app_repo_state, signup,
};
use tower::ServiceExt;
use uuid::Uuid;

/// Upload files into a project's `shared/knowledge/` folder. Filenames carry
/// the `knowledge/` prefix so dirent routes them into the corpus folder.
async fn upload_to_knowledge(
    app: &axum::Router,
    token: &str,
    project_id: &str,
    files: &[(&str, &[u8])],
) -> StatusCode {
    let prefixed: Vec<(String, &[u8])> = files
        .iter()
        .map(|(name, body)| (format!("knowledge/{name}"), *body))
        .collect();
    let refs: Vec<(&str, &[u8])> = prefixed.iter().map(|(n, b)| (n.as_str(), *b)).collect();
    let (boundary, body) = build_multipart_body(&refs);
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/dirents?path=projects/{project_id}/shared"))
        .header("authorization", format!("Bearer {token}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(axum::body::Body::from(body))
        .unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

/// Poll the project's corpus store until `count()` reaches `want`, or time out.
async fn wait_for_count(state: &Arc<AppState>, project_id: Uuid, want: u32) -> u32 {
    for _ in 0..100 {
        let store = state.store_for(project_id).await.expect("store_for");
        let n = store.read().await.count();
        if n == want {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let store = state.store_for(project_id).await.expect("store_for");
    let n = store.read().await.count();
    n
}

async fn personal_project_uuid(app: &axum::Router, token: &str) -> (String, Uuid) {
    let p = get_personal_project(app, token).await;
    let slug = p["slug"].as_str().unwrap().to_string();
    let id = Uuid::parse_str(p["id"].as_str().unwrap()).unwrap();
    (slug, id)
}

#[tokio::test]
async fn upload_to_knowledge_indexes_and_delete_removes() {
    let (app, _repo, state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;

    // Touch shared so the knowledge folder is created and listed.
    let _ = authed(&app, "GET", &format!("/dirents?path=projects/{pid}/shared"), &token, None).await;

    let status = upload_to_knowledge(
        &app,
        &token,
        &pid.to_string(),
        &[("note.md", b"# Freedonia\n\nThe capital is Glorkville." as &[u8])],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(wait_for_count(&state, pid, 1).await, 1, "doc should be indexed");

    // Search finds it.
    let store = state.store_for(pid).await.unwrap();
    let page = store.read().await.search("Glorkville", 0, 10).unwrap();
    assert!(!page.results.is_empty(), "search should find the document");

    // Delete the file → resync purges it.
    let del = authed(
        &app,
        "DELETE",
        &format!("/dirents/projects/{pid}/shared/knowledge/note.md"),
        &token,
        None,
    )
    .await;
    assert_eq!(del.0, StatusCode::NO_CONTENT);
    assert_eq!(wait_for_count(&state, pid, 0).await, 0, "doc should be purged");
}

#[tokio::test]
async fn duplicate_content_indexed_once() {
    let (app, _repo, state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;

    upload_to_knowledge(
        &app,
        &token,
        &pid.to_string(),
        &[
            ("a.md", b"# Same\n\nIdentical body." as &[u8]),
            ("b.md", b"# Same\n\nIdentical body." as &[u8]),
        ],
    )
    .await;
    // UUIDv5 on identical bytes collides → one document.
    assert_eq!(wait_for_count(&state, pid, 1).await, 1);
}

#[tokio::test]
async fn upload_outside_knowledge_is_not_indexed() {
    let (app, _repo, state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;

    // Upload to shared root (not knowledge).
    let (boundary, body) = build_multipart_body(&[("loose.md", b"# Loose\n\nNot indexed." as &[u8])]);
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/dirents?path=projects/{pid}/shared"))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(axum::body::Body::from(body))
        .unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);

    // Give any (incorrectly triggered) resync a chance, then assert empty.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let store = state.store_for(pid).await.unwrap();
    assert_eq!(store.read().await.count(), 0, "files outside knowledge must not be indexed");
}

#[tokio::test]
async fn knowledge_folder_cannot_be_deleted() {
    let (app, _repo, _state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;

    let _ = authed(&app, "GET", &format!("/dirents?path=projects/{pid}/shared"), &token, None).await;

    let del = authed(
        &app,
        "DELETE",
        &format!("/dirents/projects/{pid}/shared/knowledge"),
        &token,
        None,
    )
    .await;
    assert_eq!(del.0, StatusCode::BAD_REQUEST, "knowledge folder must be protected");
}

#[tokio::test]
async fn deleting_a_subfolder_purges_its_documents() {
    let (app, _repo, state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;

    // Two docs under a nested subfolder; recursion indexes both.
    upload_to_knowledge(
        &app,
        &token,
        &pid.to_string(),
        &[
            ("sub/one.md", b"# One\n\nAlpha content." as &[u8]),
            ("sub/two.md", b"# Two\n\nBeta content." as &[u8]),
        ],
    )
    .await;
    assert_eq!(wait_for_count(&state, pid, 2).await, 2, "both nested docs indexed");

    // Delete the whole subfolder → resync purges every doc that lived under it.
    let del = authed(
        &app,
        "DELETE",
        &format!("/dirents/projects/{pid}/shared/knowledge/sub"),
        &token,
        None,
    )
    .await;
    assert_eq!(del.0, StatusCode::NO_CONTENT);
    assert_eq!(wait_for_count(&state, pid, 0).await, 0, "subfolder docs purged");
}

#[tokio::test]
async fn knowledge_folder_cannot_be_moved_or_renamed() {
    let (app, _repo, _state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;
    let _ = authed(&app, "GET", &format!("/dirents?path=projects/{pid}/shared"), &token, None).await;

    let body = serde_json::json!({
        "op": "move",
        "sources": [format!("projects/{pid}/shared/knowledge")],
        "destination": format!("projects/{pid}/shared"),
        "new_name": "notes",
    });
    let (status, json) = authed(&app, "PATCH", "/dirents", &token, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    // Rejected per-item (the batch returns 200 but the source lands in `failed`,
    // not `succeeded`), so the folder is never moved or renamed.
    let failed = json["failed"].as_array().expect("failed array");
    assert_eq!(failed.len(), 1, "knowledge root must be rejected: {json}");
    assert!(json["succeeded"].as_array().map(|a| a.is_empty()).unwrap_or(false));
}

#[tokio::test]
async fn knowledge_folder_cannot_be_copied() {
    let (app, _repo, _state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;
    let _ = authed(&app, "GET", &format!("/dirents?path=projects/{pid}/shared"), &token, None).await;

    let body = serde_json::json!({
        "op": "copy",
        "sources": [format!("projects/{pid}/shared/knowledge")],
        "destination": format!("projects/{pid}/shared"),
    });
    let (status, json) = authed(&app, "PATCH", "/dirents", &token, Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    let failed = json["failed"].as_array().expect("failed array");
    assert_eq!(failed.len(), 1, "copying knowledge root must be rejected: {json}");
}

#[tokio::test]
async fn knowledge_files_reports_per_file_indexed_status() {
    let (app, _repo, state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;
    let _ = authed(&app, "GET", &format!("/dirents?path=projects/{pid}/shared"), &token, None).await;

    let status = upload_to_knowledge(
        &app,
        &token,
        &pid.to_string(),
        &[("note.md", b"# Freedonia\n\nThe capital is Glorkville." as &[u8])],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(wait_for_count(&state, pid, 1).await, 1, "doc should be indexed");

    let (code, json) = authed(
        &app,
        "GET",
        &format!("/projects/{pid}/knowledge/files"),
        &token,
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let files = json["files"].as_array().expect("files array");
    let note = files
        .iter()
        .find(|f| f["path"] == "knowledge/note.md")
        .expect("note.md present in knowledge files");
    assert_eq!(note["indexed"], true, "uploaded file should report indexed: {json}");
}

/// Overlapping rescans must converge: each upload spawns a background rescan and
/// they run concurrently. Per-project serialization makes the last-queued one
/// scan the final folder, so every uploaded file ends indexed — an unserialized
/// rescan could let an older scan's set purge a file a newer scan just added.
#[tokio::test]
async fn concurrent_uploads_all_indexed() {
    let (app, _repo, state) = make_app_repo_state().await;
    let username = format!("u_{}", Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let (_slug, pid) = personal_project_uuid(&app, &token).await;

    // Create the knowledge folder and pre-open the store so the concurrent
    // rescans reuse one cached handle (avoids the Store::new dir-creation race).
    let _ = authed(&app, "GET", &format!("/dirents?path=projects/{pid}/shared"), &token, None).await;
    let _ = state.store_for(pid).await.expect("store_for");

    let n: u32 = 8;
    let mut handles = Vec::new();
    for i in 0..n {
        let app = app.clone();
        let token = token.clone();
        let pid_s = pid.to_string();
        handles.push(tokio::spawn(async move {
            let name = format!("doc{i}.md");
            let body = format!("# Doc {i}\n\nUnique body number {i}.");
            upload_to_knowledge(&app, &token, &pid_s, &[(name.as_str(), body.as_bytes())]).await
        }));
    }
    for h in handles {
        assert_eq!(h.await.unwrap(), StatusCode::OK, "each upload should succeed");
    }

    assert_eq!(
        wait_for_count(&state, pid, n).await,
        n,
        "every concurrently-uploaded file must end up indexed"
    );
}
