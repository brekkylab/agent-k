use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use agent_k_backend::model::{SendMessageRequest, SendMessageResponse, SessionResponse};
use agent_k_backend::router::{get_router, speedwagon_store};
use agent_k_backend::state::AppState;
use ailoy::agent::default_provider_mut;
use speedwagon::FileType;

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

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn test_ingest_message_purge_cycle() {
    dotenvy::dotenv().ok();

    {
        let mut provider = default_provider_mut().await;
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            provider.model_openai(key);
        }
    }

    let store = speedwagon_store();
    let test_content = b"The capital of Freedonia is Glorkville. This is a unique fact.";
    let doc_id = store
        .write()
        .await
        .ingest(test_content.iter().copied(), FileType::MD)
        .await
        .expect("ingest failed");

    let state = Arc::new(AppState::new());
    let app = get_router(state);

    // Create session
    let resp = app
        .clone()
        .oneshot(json_request("POST", "/sessions", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let session: SessionResponse = serde_json::from_slice(&body).unwrap();
    let session_id = session.id;

    // Send message about the ingested document
    let msg_body = serde_json::to_string(&SendMessageRequest {
        content: "What is the capital of Freedonia?".into(),
    })
    .unwrap();
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
    let msg_resp: SendMessageResponse = serde_json::from_slice(&body).unwrap();

    assert!(!msg_resp.messages.is_empty(), "messages should not be empty");
    assert!(
        !msg_resp.final_content.is_empty(),
        "final_content should not be empty"
    );
    assert!(
        msg_resp
            .messages
            .iter()
            .any(|m| m.role == ailoy::message::Role::Assistant),
        "should contain at least one assistant message"
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
    let post_purge: SendMessageResponse = serde_json::from_slice(&body).unwrap();
    assert!(!post_purge.final_content.is_empty());
}

#[tokio::test]
async fn test_send_message_to_nonexistent_session() {
    let state = Arc::new(AppState::new());
    let app = get_router(state);

    let fake_id = uuid::Uuid::new_v4();
    let resp = app
        .oneshot(json_request(
            "POST",
            &format!("/sessions/{fake_id}/messages"),
            Some(r#"{"content":"hello"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
