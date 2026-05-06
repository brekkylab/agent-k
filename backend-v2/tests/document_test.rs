#[path = "common/mod.rs"]
mod common;

use axum::http::StatusCode;
use common::{
    bulk_purge_documents, get_document, ingest_document, ingest_documents_with_status,
    list_documents, make_app_with_repo, make_repo, purge_document,
};

#[tokio::test]
async fn list_documents_empty_initially() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let docs = list_documents(&app).await;
    assert!(docs.is_empty());
}

#[tokio::test]
async fn ingest_and_list_document() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let content = b"# Test Document\n\nThis is test content for indexing.";
    let doc = ingest_document(&app, "test.md", content).await;

    assert!(doc.get("id").is_some(), "response should contain id");
    assert!(doc.get("title").is_some(), "response should contain title");
    assert!(doc.get("len").is_some(), "response should contain len");

    let docs = list_documents(&app).await;
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["id"], doc["id"]);
}

#[tokio::test]
async fn get_document_by_id() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let content = b"# Getting by ID\n\nSome content here.";
    let created = ingest_document(&app, "get-test.md", content).await;
    let id = created["id"].as_str().unwrap();

    let (status, fetched) = get_document(&app, id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["id"].as_str().unwrap(), id);
    assert_eq!(fetched["title"].as_str(), created["title"].as_str());
}

#[tokio::test]
async fn get_nonexistent_document_returns_404() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let fake_id = uuid::Uuid::new_v4();
    let (status, _) = get_document(&app, &fake_id.to_string()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn purge_document_removes_it() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let content = b"# To Be Purged\n\nThis document will be deleted.";
    let doc = ingest_document(&app, "purge-me.md", content).await;
    let id = doc["id"].as_str().unwrap();

    let status = purge_document(&app, id).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let docs = list_documents(&app).await;
    assert!(docs.is_empty(), "document list should be empty after purge");

    let (status, _) = get_document(&app, id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn purge_nonexistent_returns_404() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let fake_id = uuid::Uuid::new_v4();
    let status = purge_document(&app, &fake_id.to_string()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ingest_duplicate_returns_same_id() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let content = b"# Duplicate Test\n\nSame content, same ID.";
    let doc1 = ingest_document(&app, "dup1.md", content).await;
    let doc2 = ingest_document(&app, "dup2.md", content).await;

    assert_eq!(
        doc1["id"].as_str().unwrap(),
        doc2["id"].as_str().unwrap(),
        "same content should produce same UUID (UUIDv5)"
    );

    let docs = list_documents(&app).await;
    assert_eq!(
        docs.len(),
        1,
        "duplicate ingest should not create extra document"
    );
}

// ── Multi-file ingest tests ──────────────────────────────────────────────────

#[tokio::test]
async fn ingest_multiple_documents() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let files: &[(&str, &[u8])] = &[
        ("doc1.md", b"# Document One\n\nFirst document."),
        ("doc2.txt", b"# Document Two\n\nSecond document."),
    ];

    let (status, batch) = ingest_documents_with_status(&app, files).await;
    assert_eq!(status, StatusCode::CREATED);

    let succeeded = batch["succeeded"].as_array().unwrap();
    assert_eq!(succeeded.len(), 2);
    assert!(batch["failed"].as_array().unwrap().is_empty());

    let docs = list_documents(&app).await;
    assert_eq!(docs.len(), 2);
}

#[tokio::test]
async fn ingest_partial_failure_mixed_filetypes() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let files: &[(&str, &[u8])] = &[
        ("good.md", b"# Good Document\n\nValid markdown."),
        ("bad.csv", b"a,b,c\n1,2,3"),
        ("also-good.txt", b"# Another Good\n\nAlso valid."),
    ];

    let (status, batch) = ingest_documents_with_status(&app, files).await;
    assert_eq!(status, StatusCode::OK, "partial success should return 200");

    let succeeded = batch["succeeded"].as_array().unwrap();
    let failed = batch["failed"].as_array().unwrap();
    assert_eq!(succeeded.len(), 2);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["name"].as_str().unwrap(), "bad.csv");
}

#[tokio::test]
async fn ingest_all_unsupported_returns_empty_succeeded() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let files: &[(&str, &[u8])] = &[("data.csv", b"a,b,c")];

    let (status, batch) = ingest_documents_with_status(&app, files).await;
    assert_eq!(status, StatusCode::OK);

    let succeeded = batch["succeeded"].as_array().unwrap();
    let failed = batch["failed"].as_array().unwrap();
    assert!(succeeded.is_empty());
    assert_eq!(failed.len(), 1);
}

#[tokio::test]
async fn ingest_no_file_field_returns_400() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let boundary = "----testboundary";
    let body = format!("--{boundary}--\r\n");

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/documents")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(axum::body::Body::from(body))
        .unwrap();

    let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── Bulk purge tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn bulk_purge_multiple_documents() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let doc1 = ingest_document(&app, "a.md", b"# Doc A\n\nContent A.").await;
    let doc2 = ingest_document(&app, "b.md", b"# Doc B\n\nContent B.").await;
    let id1 = doc1["id"].as_str().unwrap();
    let id2 = doc2["id"].as_str().unwrap();

    let (status, resp) = bulk_purge_documents(&app, &[id1, id2]).await;
    assert_eq!(status, StatusCode::OK);

    let purged = resp["purged"].as_array().unwrap();
    assert_eq!(purged.len(), 2);
    assert!(resp["failed"].as_array().unwrap().is_empty());

    let docs = list_documents(&app).await;
    assert!(docs.is_empty());
}

#[tokio::test]
async fn bulk_purge_partial_failure() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let doc = ingest_document(&app, "c.md", b"# Doc C\n\nContent C.").await;
    let real_id = doc["id"].as_str().unwrap();
    let fake_id = uuid::Uuid::new_v4().to_string();

    let (status, resp) = bulk_purge_documents(&app, &[real_id, &fake_id]).await;
    assert_eq!(status, StatusCode::OK);

    let purged = resp["purged"].as_array().unwrap();
    let failed = resp["failed"].as_array().unwrap();
    assert_eq!(purged.len(), 1);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["name"].as_str().unwrap(), fake_id);
}

#[tokio::test]
async fn bulk_purge_empty_ids_returns_400() {
    let repo = make_repo().await;
    let app = make_app_with_repo(repo);

    let (status, _) = bulk_purge_documents(&app, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
