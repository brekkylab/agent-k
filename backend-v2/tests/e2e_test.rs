#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use agent_k_backend::{repository, router::get_router, state::AppState};
use aide::openapi::OpenApi;
use ailoy::{agent::default_provider_mut, lang_model::LangModelProvider};
use common::{extract_text, post_session, send_message, test_jwt_config};
use speedwagon::{FileType, Store, build_tools};
use tokio::sync::RwLock;

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn test_ingest_message_purge_cycle() {
    dotenvy::dotenv().ok();

    let store_path = std::env::temp_dir().join(format!("speedwagon-e2e-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(RwLock::new(
        Store::new(store_path).expect("test store init"),
    ));

    {
        let mut provider = default_provider_mut().await;
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            provider
                .models
                .insert("openai/*".into(), LangModelProvider::openai(key));
        }
        provider.tools = build_tools(store.clone());
    }

    let test_content = b"The capital of Freedonia is Glorkville. This is a unique fact.";
    let doc_id = store
        .write()
        .await
        .ingest(test_content.iter().copied(), FileType::MD)
        .await
        .expect("ingest failed");

    let repo = repository::create_repository("sqlite::memory:")
        .await
        .expect("test repo init");
    let state = Arc::new(AppState::new(repo, store.clone(), test_jwt_config()));
    let app = get_router(state).finish_api(&mut OpenApi::default());

    let session_id = post_session(&app).await;

    let outputs = send_message(&app, session_id, "What is the capital of Freedonia?").await;
    let arr = outputs.as_array().expect("response must be an array");

    assert!(!arr.is_empty(), "messages should not be empty");

    let has_assistant = arr.iter().any(|o| {
        o.get("message")
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            == Some("assistant")
    });
    assert!(
        has_assistant,
        "should contain at least one assistant message"
    );

    let text = extract_text(&outputs);
    assert!(!text.is_empty(), "assistant text should not be empty");
    assert!(
        text.contains("Glorkville"),
        "response should mention 'Glorkville' from the ingested document, got: {text}",
    );

    // Purge the document
    store.write().await.purge(doc_id).expect("purge failed");

    // Send same message after purge
    let outputs = send_message(&app, session_id, "What is the capital of Freedonia?").await;
    let post_purge_text = extract_text(&outputs);
    assert!(
        !post_purge_text.is_empty(),
        "post-purge response should not be empty"
    );
}
