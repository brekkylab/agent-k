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
use speedwagon::{Store, build_tools};
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

fn multipart_request(files: &[(&str, &[u8])]) -> Request<Body> {
    let boundary = "----e2e-test-boundary";
    let mut body = Vec::new();
    for (filename, content) in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    Request::builder()
        .method("POST")
        .uri("/documents")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap()
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

fn assert_send_ok(status: StatusCode, body: &[u8]) -> serde_json::Value {
    if status != StatusCode::OK {
        panic!(
            "send_message returned {status}: {}",
            String::from_utf8_lossy(body)
        );
    }
    serde_json::from_slice(body).unwrap()
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn test_ingest_message_purge_cycle() {
    dotenvy::dotenv().ok();

    let store_path = std::env::temp_dir().join(format!("speedwagon-e2e-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(RwLock::new(
        Store::new(&store_path).expect("test store init"),
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

    let repo = repository::create_repository("sqlite::memory:")
        .await
        .expect("test repo init");
    let state = Arc::new(AppState::new(repo, store));
    let app = get_router(state).finish_api(&mut OpenApi::default());

    // ── Ingest two documents via HTTP multipart ──────────────────────────────
    let resp = app
        .clone()
        .oneshot(multipart_request(&[
            (
                "freedonia.md",
                b"The capital of Freedonia is Glorkville. This is a unique fact.",
            ),
            (
                "zorbax.md",
                b"The largest ocean on planet Zorbax is the Shimmer Sea. It covers 40% of the surface.",
            ),
        ]))
        .await
        .unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let batch: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "batch ingest should succeed: {batch}"
    );

    let succeeded = batch["succeeded"].as_array().unwrap();
    assert_eq!(succeeded.len(), 2, "both documents should ingest");
    let doc_ids: Vec<&str> = succeeded
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();

    // ── Create session ───────────────────────────────────────────────────────
    let resp = app
        .clone()
        .oneshot(json_request("POST", "/sessions", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session_id = session["id"].as_str().unwrap();
    let msg_uri = format!("/sessions/{session_id}/messages");

    // ── Question about document 1 (Freedonia) ────────────────────────────────
    let q1 = serde_json::json!({ "content": "What is the capital of Freedonia?" }).to_string();
    let resp = app
        .clone()
        .oneshot(json_request("POST", &msg_uri, Some(&q1)))
        .await
        .unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let outputs = assert_send_ok(StatusCode::OK, &body);
    let text = extract_assistant_text(&outputs);
    assert!(
        text.contains("Glorkville"),
        "response should mention 'Glorkville', got: {text}",
    );

    // ── Question about document 2 (Zorbax) ───────────────────────────────────
    let q2 =
        serde_json::json!({ "content": "What is the largest ocean on planet Zorbax?" }).to_string();
    let resp = app
        .clone()
        .oneshot(json_request("POST", &msg_uri, Some(&q2)))
        .await
        .unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let outputs = assert_send_ok(StatusCode::OK, &body);
    let text = extract_assistant_text(&outputs);
    assert!(
        text.contains("Shimmer Sea"),
        "response should mention 'Shimmer Sea', got: {text}",
    );

    // ── Bulk purge both documents via HTTP ───────────────────────────────────
    let purge_body = serde_json::json!({ "ids": doc_ids }).to_string();
    let resp = app
        .clone()
        .oneshot(json_request("DELETE", "/documents", Some(&purge_body)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let purge_resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let purged = purge_resp["purged"].as_array().unwrap();
    assert_eq!(purged.len(), 2, "both documents should be purged");

    // ── Verify documents are gone ────────────────────────────────────────────
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/documents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let docs: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(docs.is_empty(), "document list should be empty after purge");

    // ── Post-purge question (agent should still respond, just without KB) ────
    let resp = app
        .clone()
        .oneshot(json_request("POST", &msg_uri, Some(&q1)))
        .await
        .unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let outputs = assert_send_ok(StatusCode::OK, &body);
    let post_purge_text = extract_assistant_text(&outputs);
    assert!(
        !post_purge_text.is_empty(),
        "post-purge response should not be empty",
    );
}
