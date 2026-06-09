#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use agent_k_backend::{repository, router::get_router, state::AppState};
use aide::openapi::OpenApi;
use ailoy::{agent::default_provider_mut, lang_model::LangModelProvider};
use axum::http::StatusCode;
use common::{
    authed, build_multipart_body, extract_text, get_personal_project, login, send_message, signup,
    test_jwt_config,
};
use tower::ServiceExt as _;

/// End-to-end: upload documents into a project's `knowledge` folder, let the
/// background resync index them, then ask a Speedwagon session questions that
/// can only be answered from those documents.
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn knowledge_corpus_answers_questions() {
    dotenvy::dotenv().ok();

    {
        let mut provider = default_provider_mut();
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            provider
                .models
                .insert("openai/*".into(), LangModelProvider::openai(key));
        }
    }

    let repo = repository::create_repository("sqlite::memory:")
        .await
        .expect("test repo init");
    let data_root = std::env::temp_dir().join(format!("agent-k-e2e-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo, test_jwt_config(), data_root));
    let app = get_router(state.clone()).finish_api(&mut OpenApi::default());

    // ── User, project, and a Speedwagon session ──────────────────────────────
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_slug = project["slug"].as_str().unwrap();
    let project_id = uuid::Uuid::parse_str(project["id"].as_str().unwrap()).unwrap();

    let (status, session_body) = authed(
        &app,
        "POST",
        "/sessions",
        &token,
        Some(serde_json::json!({ "project_ref": project_slug, "agent_type": "speedwagon" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create session: {session_body}");
    let session_id = uuid::Uuid::parse_str(session_body["id"].as_str().unwrap()).unwrap();

    // ── Upload two files into the knowledge folder (dirent) ──────────────────
    let files: &[(&str, &[u8])] = &[
        (
            "knowledge/freedonia.md",
            b"The capital of Freedonia is Glorkville. This is a unique fact." as &[u8],
        ),
        (
            "knowledge/zorbax.md",
            b"The largest ocean on planet Zorbax is the Shimmer Sea. It covers 40% of the surface." as &[u8],
        ),
    ];
    let (boundary, body) = build_multipart_body(files);
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/dirents?path=projects/{project_id}/shared"))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(axum::body::Body::from(body))
        .unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);

    // ── Wait for the background resync to index both documents ───────────────
    let mut indexed = 0;
    for _ in 0..200 {
        indexed = state.store_for(project_id).await.unwrap().read().await.count();
        if indexed == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(indexed, 2, "both knowledge documents should be indexed");

    // ── Question about document 1 (Freedonia) ────────────────────────────────
    let outputs = send_message(&app, session_id, "What is the capital of Freedonia?", &token).await;
    let text = extract_text(&outputs);
    assert!(text.contains("Glorkville"), "expected 'Glorkville', got: {text}");

    // ── Question about document 2 (Zorbax) ───────────────────────────────────
    let outputs =
        send_message(&app, session_id, "What is the largest ocean on planet Zorbax?", &token).await;
    let text = extract_text(&outputs);
    assert!(text.contains("Shimmer Sea"), "expected 'Shimmer Sea', got: {text}");

    // ── Delete one file → resync purges it ───────────────────────────────────
    let del = authed(
        &app,
        "DELETE",
        &format!("/dirents/projects/{project_id}/shared/knowledge/freedonia.md"),
        &token,
        None,
    )
    .await;
    assert_eq!(del.0, StatusCode::NO_CONTENT);
    let mut after = indexed;
    for _ in 0..200 {
        after = state.store_for(project_id).await.unwrap().read().await.count();
        if after == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(after, 1, "deleted document should be purged from the corpus");
}
