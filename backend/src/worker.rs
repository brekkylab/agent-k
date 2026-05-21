//! Automation worker loop. Picks up queued runs and executes prompts
//! against the router agent in the run's session.
//!
//! Crash safety: each claimed run has its `lease_until` heartbeated by a
//! companion task. If a heartbeat finds the row no longer ours (housekeeper
//! requeued it) it cancels the in-flight agent. A separate housekeeper task
//! periodically requeues expired-lease rows and NULLs idempotency keys past
//! their retention window.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ailoy::message::{Message, MessageOutput, Part, Role};
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    cron::next_fire_after,
    handlers::session::{attribute_messages, build_agent, build_sandbox},
    model::{EventKind, RunStatus, TriggerSpec},
    repository::DbAutomationRun,
    state::AppState,
};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const REAP_INTERVAL: Duration = Duration::from_secs(60);
const IDEMPOTENCY_CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);
const CRON_TICK_INTERVAL: Duration = Duration::from_secs(15);
const LEASE_MINUTES: i64 = 3;
const WEBHOOK_IDEMPOTENCY_RETENTION_HOURS: i64 = 24;

// Retry policy. MAX_ATTEMPTS is the total number of attempts including the
// initial one — so RETRY_BACKOFFS has MAX_ATTEMPTS-1 entries (one per gap
// between consecutive attempts). Attempt N (1-indexed) failing schedules
// attempt N+1 with backoff RETRY_BACKOFFS[N-1].
const MAX_ATTEMPTS: i64 = 3;
const RETRY_BACKOFFS: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(120)];

/// Spawn `count` independent worker tasks. Each loops claim → execute.
pub fn spawn_workers(state: Arc<AppState>, count: usize) {
    for idx in 0..count {
        let state = state.clone();
        tokio::spawn(async move { worker_loop(state, idx).await });
    }
    tracing::info!(count, "automation workers spawned");
}

/// Recover leftover `running` rows from prior crashes, then loop two
/// independent periodic chores:
///
/// 1. **Reap** expired-lease `running` rows back to `queued` every
///    `REAP_INTERVAL`.
/// 2. **Cleanup** idempotency keys older than
///    `WEBHOOK_IDEMPOTENCY_RETENTION_HOURS` every
///    `IDEMPOTENCY_CLEANUP_INTERVAL`, releasing the UNIQUE-partial-index slot
///    so callers can reuse the same `Idempotency-Key` after the window.
///
/// Must be spawned once per process.
pub fn spawn_housekeeper(state: Arc<AppState>) {
    tokio::spawn(async move {
        match state.repository.reap_all_running().await {
            Ok(reaped) if !reaped.is_empty() => {
                tracing::warn!(
                    count = reaped.len(),
                    "boot reap: requeued orphaned running rows"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::error!("boot reap failed: {e}"),
        }

        let mut reap_tick = tokio::time::interval(REAP_INTERVAL);
        reap_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Discard the immediate first reap tick — boot reap above already ran.
        reap_tick.tick().await;
        let mut cleanup_tick = tokio::time::interval(IDEMPOTENCY_CLEANUP_INTERVAL);
        cleanup_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Keep the immediate first cleanup tick — sweep on boot, then on cadence.

        loop {
            tokio::select! {
                _ = reap_tick.tick() => {
                    if let Err(e) = reap_once(&state).await {
                        tracing::error!("reap failed: {e}");
                    }
                }
                _ = cleanup_tick.tick() => {
                    if let Err(e) = idempotency_cleanup_once(&state, Utc::now()).await {
                        tracing::error!("idempotency cleanup failed: {e}");
                    }
                }
            }
        }
    });
}

async fn reap_once(state: &Arc<AppState>) -> Result<(), String> {
    let reaped = state
        .repository
        .reap_expired_leases(Utc::now())
        .await
        .map_err(|e| e.to_string())?;
    for run_id in reaped {
        tracing::warn!(run = %run_id, "lease expired — requeued");
    }
    Ok(())
}

async fn idempotency_cleanup_once(
    state: &Arc<AppState>,
    now: chrono::DateTime<Utc>,
) -> Result<(), String> {
    let cutoff = now - chrono::Duration::hours(WEBHOOK_IDEMPOTENCY_RETENTION_HOURS);
    let cleared = state
        .repository
        .clear_expired_idempotency_keys(cutoff)
        .await
        .map_err(|e| e.to_string())?;
    if cleared > 0 {
        tracing::info!(cleared, "idempotency keys NULL'd past retention");
    }
    Ok(())
}

/// Periodically scans for cron triggers whose `next_fire_at` has elapsed and
/// fires them via the atomic `fire_cron_trigger` repo method.
pub fn spawn_cron_ticker(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CRON_TICK_INTERVAL).await;
            if let Err(e) = cron_tick_once(&state, Utc::now()).await {
                tracing::error!("cron tick failed: {e}");
            }
        }
    });
}

async fn cron_tick_once(state: &Arc<AppState>, now: chrono::DateTime<Utc>) -> Result<(), String> {
    let due = state
        .repository
        .list_due_cron_triggers(now)
        .await
        .map_err(|e| e.to_string())?;
    for trigger in due {
        if let Err(e) = fire_cron_trigger_once(state, &trigger, now).await {
            tracing::error!(trigger = %trigger.id, "cron fire failed: {e}");
        }
    }
    Ok(())
}

async fn fire_cron_trigger_once(
    state: &Arc<AppState>,
    trigger: &crate::repository::DbAutomationTrigger,
    now: chrono::DateTime<Utc>,
) -> Result<(), String> {
    let automation = state
        .repository
        .get_automation(trigger.automation_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("automation {} not found", trigger.automation_id))?;
    let spec = TriggerSpec::from_db(trigger.kind, &trigger.spec_json)
        .map_err(|e| format!("trigger spec decode: {e}"))?;
    let TriggerSpec::Cron { expr, tz } = &spec else {
        return Err("non-cron trigger surfaced in due list".into());
    };
    let default_tz = crate::cron::default_tz_name();
    let tz_name = tz.as_deref().unwrap_or(default_tz);
    let next_fire =
        next_fire_after(expr, tz_name, now).map_err(|e| format!("compute next_fire_at: {e}"))?;
    let payload = json!({
        "source": "cron",
        "trigger_id": trigger.id.to_string(),
        "fired_for": trigger.next_fire_at,
    });
    let run = state
        .repository
        .fire_cron_trigger(
            automation.id,
            automation.project_id,
            automation.created_by,
            trigger.id,
            now,
            next_fire,
            &payload,
        )
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(trigger = %trigger.id, run = %run.id, "cron trigger fired");
    Ok(())
}

async fn worker_loop(state: Arc<AppState>, idx: usize) {
    loop {
        match try_claim_and_execute(&state).await {
            Ok(true) => continue,
            Ok(false) => tokio::time::sleep(POLL_INTERVAL).await,
            Err(e) => {
                tracing::error!(worker = idx, "claim failed: {e}");
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }
}

async fn try_claim_and_execute(state: &Arc<AppState>) -> Result<bool, String> {
    let now = Utc::now();
    let lease_until = now + chrono::Duration::minutes(LEASE_MINUTES);
    let claimed = state
        .repository
        .claim_due_run(now, lease_until)
        .await
        .map_err(|e| e.to_string())?;
    let Some(run) = claimed else { return Ok(false) };

    tracing::info!(run = %run.id, "claimed run");

    // attempt is a derived property (chain depth via previous_run_id), not a
    // persisted column. Compute once per claim and cache locally — the worker
    // uses it for event tagging and retry budget decisions.
    let attempt = state
        .repository
        .compute_run_attempt(run.id)
        .await
        .map_err(|e| e.to_string())?;

    let cancel = CancellationToken::new();
    // Belt: panic anywhere inside this scope unwinds the drop_guard → cancel.
    let _drop_guard = cancel.clone().drop_guard();
    let lease_lost = Arc::new(AtomicBool::new(false));
    let mut heartbeat = spawn_heartbeat(state.clone(), run.id, cancel.clone(), lease_lost.clone());

    // Race execute_run against the heartbeat task; whichever ends first drives the flow.
    let agent_result = tokio::select! {
        // Happy path: agent ran to completion (Ok or Err). Signal HB to exit and keep the result.
        result = execute_run(state, &run, &cancel) => {
            cancel.cancel();
            Some(result)
        }
        // Failure path: HB ended first — lease-loss self-cancel (#4), panic (#7), or cancel-ack.
        // The agent future is dropped right here; we skip finalize and let the reaper own the row.
        hb_result = &mut heartbeat => {
            cancel.cancel();
            if let Err(je) = &hb_result {
                if je.is_panic() {
                    tracing::error!(run = %run.id, "heartbeat task panicked");
                }
            }
            None
        }
    };

    // Happy path: wait for HB to observe the cancel and exit. Failure path: already done, returns now.
    let _ = heartbeat.await;

    if agent_result.is_none() || lease_lost.load(Ordering::SeqCst) {
        tracing::warn!(run = %run.id, "abandoning run (heartbeat ended or lease lost)");
        return Ok(true);
    }

    let result = agent_result.expect("agent_result branch checked above");
    let (final_status, kind, payload) = match &result {
        Ok(()) => {
            tracing::info!(run = %run.id, "run succeeded");
            (RunStatus::Succeeded, EventKind::Succeeded, None)
        }
        Err(e) => {
            tracing::warn!(run = %run.id, "run failed: {e}");
            (
                RunStatus::Failed,
                EventKind::Failed,
                Some(json!({ "error": e })),
            )
        }
    };

    let finalize_owned = match state
        .repository
        .finalize_run(run.id, final_status, kind, payload.as_ref())
        .await
    {
        Ok(true) => true,
        Ok(false) => {
            tracing::warn!(run = %run.id, "finalize_run found row no longer running (reaper raced us)");
            false
        }
        Err(e) => {
            tracing::error!(run = %run.id, "failed to finalize run: {e}");
            false
        }
    };

    // Schedule a retry when (a) we actually finalized as Failed, and (b) the
    // attempt is still within budget. The reaper-raced case (finalize_owned ==
    // false) skips retry because the row is back to queued under another
    // worker — that retry happens via lease loss + re-claim, not via a new
    // chain entry.
    if finalize_owned
        && matches!(final_status, RunStatus::Failed)
        && let Some(backoff) = retry_backoff_for(attempt)
    {
        let scheduled_for = Utc::now() + chrono::Duration::from_std(backoff).unwrap_or_default();
        let next_attempt = attempt + 1;
        match state
            .repository
            .schedule_retry(&run, scheduled_for, next_attempt)
            .await
        {
            Ok(retry_run) => {
                tracing::info!(
                    run = %run.id, next = %retry_run.id, attempt = next_attempt,
                    "retry scheduled",
                );
            }
            Err(e) => {
                tracing::error!(run = %run.id, "schedule_retry failed: {e}");
            }
        }
    }

    Ok(true)
}

/// Returns the wait before the next attempt given the just-failed attempt
/// number. None when we've exhausted `MAX_ATTEMPTS`.
fn retry_backoff_for(current_attempt: i64) -> Option<Duration> {
    if current_attempt >= MAX_ATTEMPTS {
        return None;
    }
    let idx = (current_attempt - 1).max(0) as usize;
    RETRY_BACKOFFS.get(idx).copied()
}

fn spawn_heartbeat(
    state: Arc<AppState>,
    run_id: Uuid,
    cancel: CancellationToken,
    lease_lost: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(HEARTBEAT_INTERVAL) => {}
            }
            let new_lease = Utc::now() + chrono::Duration::minutes(LEASE_MINUTES);
            match state.repository.renew_lease(run_id, new_lease).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(run = %run_id, "lease lost — cancelling agent");
                    lease_lost.store(true, Ordering::SeqCst);
                    cancel.cancel();
                    return;
                }
                Err(e) => {
                    // Transient DB error — keep retrying until next tick. Lease
                    // will eventually expire if these keep failing, at which
                    // point the reaper takes over.
                    tracing::error!(run = %run_id, "heartbeat renew error: {e}");
                }
            }
        }
    })
}

async fn execute_run(
    state: &Arc<AppState>,
    run: &DbAutomationRun,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let repo = &state.repository;
    let automation = repo
        .get_automation(run.automation_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("automation {} not found", run.automation_id))?;

    repo.append_event(run.id, EventKind::Started, None)
        .await
        .map_err(|e| e.to_string())?;

    let sandbox = build_sandbox(state, automation.project_id, run.session_id).await?;
    let agent = build_agent(sandbox).await?;
    state.insert_agent(run.session_id, agent);

    for (idx, prompt) in automation.prompts.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let step_index = idx as i64;
        repo.append_event(
            run.id,
            EventKind::StepStarted,
            Some(&json!({ "step_index": step_index })),
        )
        .await
        .map_err(|e| e.to_string())?;

        let agent_arc = state
            .get_agent(&run.session_id)
            .ok_or_else(|| format!("agent missing for session {}", run.session_id))?;
        let mut agent = agent_arc.lock().await;
        let prev_len = agent.get_history().len();

        let msg = Message::new(Role::User).with_contents([Part::text(prompt.clone())]);
        let mut stream = agent.run(msg);
        let mut outputs: Vec<MessageOutput> = Vec::new();
        let mut step_err: Option<String> = None;
        let mut cancelled = false;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    cancelled = true;
                    break;
                }
                next = stream.next() => {
                    match next {
                        None => break,
                        Some(Ok(o)) => {
                            outputs.push(o);
                            continue;
                        }
                        Some(Err(e)) => {
                            step_err = Some(e.to_string());
                            break;
                        }
                    }
                }
            }
        }
        drop(stream);

        let new_msgs = agent.get_history()[prev_len..].to_vec();
        drop(agent);

        // Attribute the prompt to the automation's creator (User-kind) and any
        // agent outputs to the agent (Agent-kind), via the same helper that
        // session.send_message uses.
        let to_persist = attribute_messages(new_msgs, &outputs, automation.created_by);
        repo.append_messages(run.session_id, &to_persist)
            .await
            .map_err(|e| e.to_string())?;

        if cancelled {
            return Err("cancelled".into());
        }
        if let Some(err) = step_err {
            return Err(format!("step {step_index} failed: {err}"));
        }

        repo.append_event(
            run.id,
            EventKind::StepFinished,
            Some(&json!({ "step_index": step_index })),
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}
