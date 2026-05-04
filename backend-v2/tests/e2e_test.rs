use std::sync::Arc;

use aide::openapi::OpenApi;
use ailoy::lang_model::LangModelProvider;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use agent_k_backend::repository;
use agent_k_backend::router::get_router;
use agent_k_backend::state::AppState;
use ailoy::agent::default_provider_mut;
use speedwagon::{FileType, Store, build_tools};
use tokio::sync::RwLock;

fn json_request(method: &str, uri: &str, body: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    match body {
        Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
        None => builder.body(Body::from("{}")).unwrap(),
    }
}

fn extract_assistant_text(outputs: &serde_json::Value) -> String {
    outputs
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let depth = o.get("depth").and_then(|d| d.as_u64()).unwrap_or(0);
                    if depth != 0 {
                        return None;
                    }
                    o.get("message")?
                        .get("contents")?
                        .as_array()?
                        .iter()
                        .filter_map(|p| p.get("text")?.as_str())
                        .map(str::to_string)
                        .reduce(|a, b| a + &b)
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

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
    let state = Arc::new(AppState::new(repo, store.clone()));
    let app = get_router(state).finish_api(&mut OpenApi::default());

    // Create session
    let resp = app
        .clone()
        .oneshot(json_request("POST", "/sessions", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session_id = session["id"].as_str().unwrap();

    // Send message about the ingested document
    let msg_body =
        serde_json::json!({ "content": "What is the capital of Freedonia?" }).to_string();
    let resp = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/sessions/{session_id}/messages"),
            Some(&msg_body),
        ))
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    if status != StatusCode::OK {
        let body_str = String::from_utf8_lossy(&body);
        panic!("send_message returned {status}: {body_str}");
    }
    let outputs: serde_json::Value = serde_json::from_slice(&body).unwrap();
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

    let text = extract_assistant_text(&outputs);
    assert!(!text.is_empty(), "assistant text should not be empty");
    assert!(
        text.contains("Glorkville"),
        "response should mention 'Glorkville' from the ingested document, got: {text}",
    );

    // Purge the document
    store.write().await.purge(doc_id).expect("purge failed");

    // Send same message after purge
    let resp = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/sessions/{session_id}/messages"),
            Some(&msg_body),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let post_purge: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let post_purge_text = extract_assistant_text(&post_purge);
    assert!(
        !post_purge_text.is_empty(),
        "post-purge response should not be empty"
    );
}
