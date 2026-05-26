//! Multi-tenancy unit tests for the per-project Speedwagon store and the
//! ingest_jobs / project_documents repository tables.

#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use agent_k_backend::{model::IngestStatus, state::AppState};
use chrono::Utc;
use uuid::Uuid;

fn temp_data_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("agent-k-mt-{}", Uuid::new_v4()))
}

#[tokio::test]
async fn get_store_returns_same_instance_for_same_project() {
    let repo = common::make_repo().await;
    let state = Arc::new(AppState::new(repo, common::test_jwt_config(), temp_data_root()));
    let project_id = Uuid::new_v4();

    let s1 = state.get_store(project_id).await;
    let s2 = state.get_store(project_id).await;
    assert!(
        Arc::ptr_eq(&s1, &s2),
        "second call should hand out the cached Arc"
    );
}

#[tokio::test]
async fn get_store_returns_distinct_instances_per_project() {
    let repo = common::make_repo().await;
    let state = Arc::new(AppState::new(repo, common::test_jwt_config(), temp_data_root()));
    let s1 = state.get_store(Uuid::new_v4()).await;
    let s2 = state.get_store(Uuid::new_v4()).await;
    assert!(
        !Arc::ptr_eq(&s1, &s2),
        "different project ids must yield different stores"
    );
}

#[tokio::test]
async fn enqueue_creates_queued_job() {
    let repo = common::make_repo().await;
    let project_id = seed_project(&repo).await;

    let job = repo
        .enqueue_ingest_job(project_id, "alpha.pdf")
        .await
        .expect("enqueue");
    assert_eq!(job.project_id, project_id);
    assert_eq!(job.source_path, "alpha.pdf");
    assert_eq!(job.status, IngestStatus::Queued);
    assert_eq!(job.attempts, 0);
}

#[tokio::test]
async fn enqueue_is_idempotent_per_path() {
    let repo = common::make_repo().await;
    let project_id = seed_project(&repo).await;

    let a = repo
        .enqueue_ingest_job(project_id, "doc.md")
        .await
        .expect("first enqueue");
    let b = repo
        .enqueue_ingest_job(project_id, "doc.md")
        .await
        .expect("second enqueue");
    assert_eq!(a.id, b.id, "re-enqueue must update the same row");

    let jobs = repo
        .list_ingest_jobs_for_project(project_id)
        .await
        .expect("list");
    assert_eq!(jobs.len(), 1, "no duplicate row should be created");
}

#[tokio::test]
async fn claim_then_finalize_done_writes_project_document() {
    let repo = common::make_repo().await;
    let project_id = seed_project(&repo).await;

    let _enqueued = repo
        .enqueue_ingest_job(project_id, "report.pdf")
        .await
        .expect("enqueue");

    let claimed = repo
        .claim_ingest_job(Utc::now() + chrono::Duration::minutes(5))
        .await
        .expect("claim")
        .expect("a job should be available");
    assert_eq!(claimed.status, IngestStatus::Running);
    assert_eq!(claimed.attempts, 1);

    let document_id = Uuid::new_v4().to_string();
    let ok = repo
        .finalize_ingest_done(claimed.id, project_id, "report.pdf", &document_id)
        .await
        .expect("finalize");
    assert!(ok, "running row should transition to done");

    let pd = repo
        .get_project_document(project_id, "report.pdf")
        .await
        .expect("get_project_document")
        .expect("mapping must exist after finalize");
    assert_eq!(pd.document_id, document_id);
    assert_eq!(pd.project_id, project_id);
}

#[tokio::test]
async fn delete_project_document_returns_document_id_and_clears_rows() {
    let repo = common::make_repo().await;
    let project_id = seed_project(&repo).await;

    repo.enqueue_ingest_job(project_id, "x.txt")
        .await
        .expect("enqueue");
    let job = repo
        .claim_ingest_job(Utc::now() + chrono::Duration::minutes(5))
        .await
        .expect("claim")
        .expect("a job");
    let doc_id = Uuid::new_v4().to_string();
    repo.finalize_ingest_done(job.id, project_id, "x.txt", &doc_id)
        .await
        .expect("finalize");

    let returned = repo
        .delete_project_document(project_id, "x.txt")
        .await
        .expect("delete");
    assert_eq!(returned.as_deref(), Some(doc_id.as_str()));

    let missing = repo
        .get_project_document(project_id, "x.txt")
        .await
        .expect("get");
    assert!(missing.is_none(), "mapping should be gone");

    let jobs = repo
        .list_ingest_jobs_for_project(project_id)
        .await
        .expect("list");
    assert!(jobs.is_empty(), "ingest_jobs row should be cleared too");
}

#[tokio::test]
async fn reap_requeues_expired_lease() {
    let repo = common::make_repo().await;
    let project_id = seed_project(&repo).await;

    repo.enqueue_ingest_job(project_id, "y.md")
        .await
        .expect("enqueue");
    let past = Utc::now() - chrono::Duration::minutes(10);
    let claimed = repo
        .claim_ingest_job(past)
        .await
        .expect("claim")
        .expect("a job");
    assert_eq!(claimed.status, IngestStatus::Running);

    let count = repo
        .reap_expired_ingest_jobs(Utc::now())
        .await
        .expect("reap");
    assert_eq!(count, 1, "the expired-lease row should be reaped");

    let jobs = repo
        .list_ingest_jobs_for_project(project_id)
        .await
        .expect("list");
    assert_eq!(jobs[0].status, IngestStatus::Queued);
    assert!(jobs[0].lease_until.is_none());
}

#[tokio::test]
async fn is_ingestable_filename_matches_supported_extensions() {
    use agent_k_backend::model::is_ingestable_filename;
    assert!(is_ingestable_filename("a.pdf"));
    assert!(is_ingestable_filename("a.PDF"));
    assert!(is_ingestable_filename("notes.md"));
    assert!(is_ingestable_filename("doc.markdown"));
    assert!(is_ingestable_filename("plain.txt"));
    assert!(!is_ingestable_filename("data.csv"));
    assert!(!is_ingestable_filename("photo.png"));
    assert!(!is_ingestable_filename("noext"));
}

/// Seed a minimal user + project so that ingest_jobs FK constraints hold.
async fn seed_project(repo: &agent_k_backend::repository::AppRepository) -> Uuid {
    use agent_k_backend::{auth::Role, repository::NewUser};

    let user_id = Uuid::new_v4();
    let username = format!("tester_{}", Uuid::new_v4().simple());
    let user = repo
        .create_user(NewUser {
            id: user_id,
            username,
            password_hash: "test-bcrypt-hash-placeholder".into(),
            role: Role::User,
            display_name: None,
            is_active: true,
        })
        .await
        .expect("create_user");

    let project = repo
        .create_project(format!("proj-{}", Uuid::new_v4().simple()), None, user.id)
        .await
        .expect("create_project");

    project.id
}
