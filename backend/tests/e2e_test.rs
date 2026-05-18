#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use agent_k::{agents::build_tools, knowledge_base::Store};
use agent_k_backend::{repository, router::get_router, state::AppState};
use aide::openapi::OpenApi;
use ailoy::{agent::default_provider_mut, lang_model::LangModelProvider};
use axum::http::StatusCode;
use common::{
    bulk_purge_documents, extract_text, get_personal_project, ingest_documents, list_documents,
    login, post_session_authed, send_message, signup, test_jwt_config,
};
use tokio::sync::RwLock;

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn test_ingest_message_purge_cycle() {
    dotenvy::dotenv().ok();

    let store_path = std::env::temp_dir().join(format!("agent-k-e2e-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(RwLock::new(
        Store::new(store_path).expect("test store init"),
    ));

    {
        let mut provider = default_provider_mut();
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            provider
                .models
                .insert("openai/*".into(), LangModelProvider::openai(key));
        }
        provider.tools = build_tools(store.clone());
    }

    let repo = repository::create_repository("sqlite::memory:")
        .await
        .expect("test repo init");
    let data_root = std::env::temp_dir().join(format!("agent-k-e2e-{}", uuid::Uuid::new_v4()));
    let state = Arc::new(AppState::new(repo, store, test_jwt_config(), data_root));
    let app = get_router(state).finish_api(&mut OpenApi::default());

    // ── Ingest two documents via multipart ───────────────────────────────────
    let batch = ingest_documents(
        &app,
        &[
            (
                "freedonia.md",
                b"The capital of Freedonia is Glorkville. This is a unique fact." as &[u8],
            ),
            (
                "zorbax.md",
                b"The largest ocean on planet Zorbax is the Shimmer Sea. It covers 40% of the surface." as &[u8],
            ),
        ],
    )
    .await;
    let succeeded = batch["succeeded"].as_array().unwrap();
    assert_eq!(succeeded.len(), 2, "both documents should ingest");
    let doc_ids: Vec<&str> = succeeded
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();

    // ── Create user and session ──────────────────────────────────────────────
    let username = format!("user_{}", uuid::Uuid::new_v4().simple());
    signup(&app, &username, "Password123!").await;
    let token = login(&app, &username, "Password123!").await;
    let project = get_personal_project(&app, &token).await;
    let project_id = project["id"].as_str().unwrap();
    let session_id = post_session_authed(&app, &token, project_id).await;

    // ── Question about document 1 (Freedonia) ────────────────────────────────
    let outputs = send_message(
        &app,
        session_id,
        "What is the capital of Freedonia?",
        &token,
    )
    .await;
    let text = extract_text(&outputs);
    assert!(
        text.contains("Glorkville"),
        "response should mention 'Glorkville', got: {text}",
    );

    // ── Question about document 2 (Zorbax) ───────────────────────────────────
    let outputs = send_message(
        &app,
        session_id,
        "What is the largest ocean on planet Zorbax?",
        &token,
    )
    .await;
    let text = extract_text(&outputs);
    assert!(
        text.contains("Shimmer Sea"),
        "response should mention 'Shimmer Sea', got: {text}",
    );

    // ── Bulk purge both documents ─────────────────────────────────────────────
    let (purge_status, purge_resp) = bulk_purge_documents(&app, &doc_ids).await;
    assert_eq!(purge_status, StatusCode::OK);
    let purged = purge_resp["purged"].as_array().unwrap();
    assert_eq!(purged.len(), 2, "both documents should be purged");

    // ── Verify documents are gone ────────────────────────────────────────────
    let docs = list_documents(&app).await;
    assert!(docs.is_empty(), "document list should be empty after purge");

    // ── Post-purge question (agent should still respond, just without KB) ────
    let outputs = send_message(
        &app,
        session_id,
        "What is the capital of Freedonia?",
        &token,
    )
    .await;
    let post_purge_text = extract_text(&outputs);
    assert!(
        !post_purge_text.is_empty(),
        "post-purge response should not be empty",
    );
}
