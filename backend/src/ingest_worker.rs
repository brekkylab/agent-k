//! Auto-ingest worker. Polls `ingest_jobs` for queued rows, reads the file
//! from disk, runs `Store::ingest`, and records the resulting document id in
//! `project_documents`. Mirrors the lease + heartbeat + reaper pattern from
//! `worker.rs` (automation), simplified — no idempotency keys, no events
//! table, no per-run cancellation.
//!
//! Crash safety: each claimed job has its `lease_until` heartbeated by the
//! same task that runs the ingest. If the task panics or the process exits,
//! the housekeeper requeues the row once the lease expires.

use std::{path::PathBuf, sync::Arc, time::Duration};

use agent_k::knowledge_base::FileType;
use chrono::Utc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{repository::DbIngestJob, state::AppState};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const REAP_INTERVAL: Duration = Duration::from_secs(60);
const LEASE_MINUTES: i64 = 5;

const MAX_ATTEMPTS: i64 = 3;

pub fn spawn_ingest_workers(state: Arc<AppState>, count: usize) {
    for idx in 0..count {
        let state = state.clone();
        tokio::spawn(async move { worker_loop(state, idx).await });
    }
    tracing::info!(count, "ingest workers spawned");
}

pub fn spawn_ingest_housekeeper(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(REAP_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Discard immediate first tick — let workers run for a moment first.
        tick.tick().await;
        loop {
            tick.tick().await;
            match state
                .repository
                .reap_expired_ingest_jobs(Utc::now())
                .await
            {
                Ok(0) => {}
                Ok(n) => tracing::warn!(count = n, "ingest reap: requeued expired jobs"),
                Err(e) => tracing::error!("ingest reap failed: {e}"),
            }
        }
    });
}

async fn worker_loop(state: Arc<AppState>, idx: usize) {
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tracing::info!(worker = idx, "ingest worker started");

    loop {
        tick.tick().await;

        let lease_until = Utc::now() + chrono::Duration::minutes(LEASE_MINUTES);
        let job = match state.repository.claim_ingest_job(lease_until).await {
            Ok(Some(j)) => j,
            Ok(None) => continue,
            Err(e) => {
                tracing::error!(worker = idx, "claim failed: {e}");
                continue;
            }
        };

        tracing::info!(
            worker = idx,
            job = %job.id,
            project = %job.project_id,
            path = %job.source_path,
            attempt = %job.attempts,
            "ingest job claimed"
        );

        let attempts = job.attempts;
        let job_id = job.id;

        let state_for_hb = state.clone();
        let hb_token = CancellationToken::new();
        let hb_token_child = hb_token.clone();
        let hb_handle = tokio::spawn(async move {
            heartbeat_loop(state_for_hb, job_id, hb_token_child).await;
        });

        let result = run_one_ingest(&state, &job).await;
        hb_token.cancel();
        let _ = hb_handle.await;

        match result {
            Ok(document_id) => {
                match state
                    .repository
                    .finalize_ingest_done(job_id, job.project_id, &job.source_path, &document_id)
                    .await
                {
                    Ok(true) => tracing::info!(
                        worker = idx,
                        job = %job_id,
                        doc = %document_id,
                        "ingest done"
                    ),
                    Ok(false) => tracing::warn!(
                        worker = idx,
                        job = %job_id,
                        "finalize_done found no running row (likely reaped)"
                    ),
                    Err(e) => tracing::error!(
                        worker = idx,
                        job = %job_id,
                        "finalize_done failed: {e}"
                    ),
                }
            }
            Err(msg) => {
                tracing::warn!(
                    worker = idx,
                    job = %job_id,
                    "ingest failed (attempt {attempts}/{MAX_ATTEMPTS}): {msg}"
                );
                if attempts >= MAX_ATTEMPTS {
                    if let Err(e2) =
                        state.repository.finalize_ingest_failed(job_id, &msg).await
                    {
                        tracing::error!(job = %job_id, "finalize_failed errored: {e2}");
                    }
                } else {
                    // Surface the lease so the reaper requeues on the next pass.
                    if let Err(e2) = state
                        .repository
                        .renew_ingest_lease(job_id, Utc::now() - chrono::Duration::seconds(1))
                        .await
                    {
                        tracing::error!(job = %job_id, "fail-requeue errored: {e2}");
                    }
                }
            }
        }
    }
}

/// Periodically renew the lease while the worker is running the ingest.
async fn heartbeat_loop(
    state: Arc<AppState>,
    job_id: Uuid,
    cancel: CancellationToken,
) {
    let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Discard immediate first tick — the claim already set a fresh lease.
    tick.tick().await;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tick.tick() => {
                let new_until = Utc::now() + chrono::Duration::minutes(LEASE_MINUTES);
                match state.repository.renew_ingest_lease(job_id, new_until).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(job = %job_id, "lease lost during heartbeat");
                        return;
                    }
                    Err(e) => {
                        tracing::error!(job = %job_id, "heartbeat errored: {e}");
                        return;
                    }
                }
            }
        }
    }
}

async fn run_one_ingest(state: &Arc<AppState>, job: &DbIngestJob) -> Result<String, String> {
    let host_path: PathBuf = state
        .data_root
        .join("projects")
        .join(job.project_id.to_string())
        .join("uploads")
        .join(&job.source_path);

    let bytes = tokio::fs::read(&host_path)
        .await
        .map_err(|e| format!("failed to read {}: {e}", host_path.display()))?;

    let filetype = parse_filetype_for_ingest(&job.source_path)
        .ok_or_else(|| format!("unsupported file type for {}", job.source_path))?;

    let store = state.get_store(job.project_id).await;
    let mut store = store.write().await;
    let document_id = store
        .ingest(bytes, filetype)
        .await
        .map_err(|e| format!("Store::ingest failed: {e}"))?;
    drop(store);

    Ok(document_id.to_string())
}

fn parse_filetype_for_ingest(filename: &str) -> Option<FileType> {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Some(FileType::PDF),
        "md" | "markdown" | "txt" => Some(FileType::MD),
        _ => None,
    }
}
