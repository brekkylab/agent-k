//! Automation worker loop. Claims queued runs and drives a **single prompt**
//! against the run's session to completion.
//!
//! Crash safety: each claimed run has its `lease_until` heartbeated by a
//! companion task. If a heartbeat finds the row no longer ours (a reaper
//! requeued it) it cancels the in-flight agent. A housekeeper periodically
//! requeues expired-lease rows. A cron ticker fires due cron triggers.
//!
//! Single-prompt note: there is no per-step cursor, so a re-claimed run clears
//! the session and re-runs its one prompt on it — no replay/orphan.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ailoy::message::Part;
use chrono::Utc;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    cron::{default_tz_name, next_fire_after},
    state::{AppState, AutomationRun, AutomationTrigger, RunLogKind, MAX_ATTEMPTS, RunStatus, TriggerSpec},
};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const REAP_INTERVAL: Duration = Duration::from_secs(60);
const CRON_TICK_INTERVAL: Duration = Duration::from_secs(15);
const LEASE_MINUTES: i64 = 3;

/// Backoffs between consecutive attempts. `RETRY_BACKOFFS[N-1]` is the wait
/// after attempt N fails; length is `MAX_ATTEMPTS - 1`.
const RETRY_BACKOFFS: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(120)];

/// Spawn the full automation runtime: `count` claim/execute workers, one
/// housekeeper (reaper), and one cron ticker. Call once at startup.
pub fn spawn_runtime(state: Arc<AppState>, count: usize) {
    for idx in 0..count {
        let state = state.clone();
        tokio::spawn(async move { worker_loop(state, idx).await });
    }
    spawn_housekeeper(state.clone());
    spawn_cron_ticker(state);
    tracing::info!(workers = count, "automation runtime spawned");
}

fn spawn_housekeeper(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Boot reap: any `running` row after a restart is orphaned.
        match state.automations.reap_expired_leases(None).await {
            Ok(reaped) if !reaped.is_empty() => {
                tracing::warn!(count = reaped.len(), "boot reap: requeued orphaned running rows");
            }
            Ok(_) => {}
            Err(e) => tracing::error!("boot reap failed: {e}"),
        }

        let mut tick = tokio::time::interval(REAP_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tick.tick().await; // discard the immediate first tick (boot reap ran)
        loop {
            tick.tick().await;
            match state.automations.reap_expired_leases(Some(Utc::now())).await {
                Ok(reaped) => {
                    for run_id in reaped {
                        tracing::warn!(run = %run_id, "lease expired — requeued");
                    }
                }
                Err(e) => tracing::error!("reap failed: {e}"),
            }
        }
    });
}

fn spawn_cron_ticker(state: Arc<AppState>) {
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
        .automations
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
    trigger: &AutomationTrigger,
    now: chrono::DateTime<Utc>,
) -> Result<(), String> {
    let automation = state
        .automations
        .get_automation(trigger.automation_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("automation {} not found", trigger.automation_id))?;
    let spec = TriggerSpec::from_db(trigger.kind, &trigger.spec_json)
        .map_err(|e| format!("trigger spec decode: {e}"))?;
    let TriggerSpec::Cron { expr, tz } = &spec else {
        return Err(format!("trigger {} is not a cron trigger", trigger.id));
    };
    let tz_name = tz.as_deref().unwrap_or(default_tz_name());
    let next_fire = next_fire_after(expr, tz_name, now)?;
    let payload = json!({
        "source": "cron",
        "trigger_id": trigger.id.to_string(),
        "fired_for": trigger.next_fire_at,
    });
    let run = state
        .automations
        .fire_cron_trigger(
            &automation,
            trigger.id,
            trigger.next_fire_at.unwrap_or(now),
            next_fire,
            &payload,
        )
        .await
        .map_err(|e| e.to_string())?;
    match run {
        Some(run) => tracing::info!(trigger = %trigger.id, run = %run.id, "cron trigger fired"),
        None => tracing::info!(trigger = %trigger.id, "cron fire skipped: automation disabled"),
    }
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
        .automations
        .claim_due_run(now, lease_until)
        .await
        .map_err(|e| e.to_string())?;
    let Some(run) = claimed else { return Ok(false) };

    tracing::info!(run = %run.id, "claimed run");

    let attempt = state
        .automations
        .compute_run_attempt(run.id)
        .await
        .map_err(|e| e.to_string())?;

    let cancel = CancellationToken::new();
    let _drop_guard = cancel.clone().drop_guard();
    let lease_lost = Arc::new(AtomicBool::new(false));
    let mut heartbeat = spawn_heartbeat(state.clone(), run.id, cancel.clone(), lease_lost.clone());

    let agent_result = tokio::select! {
        result = execute_run(state, &run, &cancel) => {
            cancel.cancel();
            Some(result)
        }
        hb_result = &mut heartbeat => {
            cancel.cancel();
            if let Err(je) = &hb_result
                && je.is_panic()
            {
                tracing::error!(run = %run.id, "heartbeat task panicked");
            }
            None
        }
    };

    let _ = heartbeat.await;

    if agent_result.is_none() || lease_lost.load(Ordering::SeqCst) {
        tracing::warn!(run = %run.id, "abandoning run (heartbeat ended or lease lost)");
        return Ok(true);
    }

    let result = agent_result.expect("agent_result branch checked above");
    let (final_status, kind, payload) = match &result {
        Ok(()) => {
            tracing::info!(run = %run.id, "run succeeded");
            (RunStatus::Succeeded, RunLogKind::Succeeded, None)
        }
        Err(e) => {
            tracing::warn!(run = %run.id, "run failed: {e}");
            (RunStatus::Failed, RunLogKind::Failed, Some(json!({ "error": e })))
        }
    };

    let finalize_owned = match state
        .automations
        .finalize_run(run.id, final_status, kind, payload.as_ref())
        .await
    {
        Ok(true) => true,
        Ok(false) => {
            tracing::warn!(run = %run.id, "finalize found row no longer running (reaper raced us)");
            false
        }
        Err(e) => {
            tracing::error!(run = %run.id, "finalize failed: {e}");
            false
        }
    };

    if finalize_owned
        && matches!(final_status, RunStatus::Failed)
        && let Some(backoff) = retry_backoff_for(attempt)
    {
        let scheduled_for = Utc::now() + chrono::Duration::from_std(backoff).unwrap_or_default();
        let next_attempt = attempt + 1;
        match state
            .automations
            .schedule_retry(&run, scheduled_for, next_attempt)
            .await
        {
            Ok(Some(retry_run)) => {
                tracing::info!(run = %run.id, next = %retry_run.id, attempt = next_attempt, "retry scheduled");
            }
            Ok(None) => tracing::info!(run = %run.id, "retry skipped: automation disabled"),
            Err(e) => tracing::error!(run = %run.id, "schedule_retry failed: {e}"),
        }
    }

    Ok(true)
}

/// Wait before the next attempt given the just-failed attempt number. `None`
/// when `MAX_ATTEMPTS` is reached.
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
            match state.automations.renew_lease(run_id, new_lease).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(run = %run_id, "lease lost — cancelling agent");
                    lease_lost.store(true, Ordering::SeqCst);
                    cancel.cancel();
                    return;
                }
                Err(e) => tracing::error!(run = %run_id, "heartbeat renew error: {e}"),
            }
        }
    })
}

async fn execute_run(
    state: &Arc<AppState>,
    run: &AutomationRun,
    cancel: &CancellationToken,
) -> Result<(), String> {
    state
        .automations
        .append_log(run.id, RunLogKind::Started, None)
        .await
        .map_err(|e| e.to_string())?;

    // The run is self-describing: it carries the rendered prompt it executes, so
    // no automation lookup is needed (and ad-hoc runs have no automation).
    let parts = vec![Part::text(run.prompt.clone())];
    state
        .sessions
        .drive_prompt(run.session_id, parts, cancel.clone())
        .await
        .map_err(|e| e.to_string())
}
