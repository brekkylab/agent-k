//! Automation worker loop. Picks up queued runs and executes prompts
//! against the router agent in the run's session.
//!
//! Crash safety: each claimed run has its `lease_until` heartbeated by a
//! companion task. If a heartbeat finds the row no longer ours (reaper
//! requeued it) it cancels the in-flight agent. A separate reaper task
//! periodically requeues expired-lease rows.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ailoy::message::{Message, Part, Role};
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    cron::next_fire_after,
    handlers::session::{build_agent, build_sandbox},
    model::{EventKind, RunStatus, TriggerSpec},
    repository::DbAutomationRun,
    state::AppState,
};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const REAP_INTERVAL: Duration = Duration::from_secs(60);
const CRON_TICK_INTERVAL: Duration = Duration::from_secs(15);
const LEASE_MINUTES: i64 = 3;

/// Spawn `count` independent worker tasks. Each loops claim → execute.
pub fn spawn_workers(state: Arc<AppState>, count: usize) {
    for idx in 0..count {
        let state = state.clone();
        tokio::spawn(async move { worker_loop(state, idx).await });
    }
    tracing::info!(count, "automation workers spawned");
}

/// Recover leftover `running` rows from prior crashes, then loop reaping
/// expired leases. Must be spawned once per process.
pub fn spawn_reaper(state: Arc<AppState>) {
    tokio::spawn(async move {
        match state.repository.reap_all_running().await {
            Ok(reaped) if !reaped.is_empty() => {
                tracing::warn!(count = reaped.len(), "boot reap: requeued orphaned running rows");
            }
            Ok(_) => {}
            Err(e) => tracing::error!("boot reap failed: {e}"),
        }
        loop {
            tokio::time::sleep(REAP_INTERVAL).await;
            if let Err(e) = reap_once(&state).await {
                tracing::error!("reap failed: {e}");
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

async fn cron_tick_once(
    state: &Arc<AppState>,
    now: chrono::DateTime<Utc>,
) -> Result<(), String> {
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
    let next_fire = next_fire_after(expr, tz_name, now)
        .map_err(|e| format!("compute next_fire_at: {e}"))?;
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

    let cancel = CancellationToken::new();
    // Belt: panic anywhere inside this scope unwinds the drop_guard → cancel.
    let _drop_guard = cancel.clone().drop_guard();
    let lease_lost = Arc::new(AtomicBool::new(false));
    let mut heartbeat =
        spawn_heartbeat(state.clone(), run.id, cancel.clone(), lease_lost.clone());

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
        Ok(()) => (RunStatus::Succeeded, EventKind::Succeeded, None),
        Err(e) => (
            RunStatus::Failed,
            EventKind::Failed,
            Some(json!({ "error": e })),
        ),
    };
    match state
        .repository
        .finalize_run(run.id, final_status, kind, 1, payload.as_ref())
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(run = %run.id, "finalize_run found row no longer running (reaper raced us)");
        }
        Err(e) => tracing::error!(run = %run.id, "failed to finalize run: {e}"),
    }

    match &result {
        Ok(()) => tracing::info!(run = %run.id, "run succeeded"),
        Err(e) => tracing::warn!(run = %run.id, "run failed: {e}"),
    }
    Ok(true)
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

    repo.append_event(run.id, EventKind::Started, 1, None)
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
            1,
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
                        Some(Ok(_)) => continue,
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

        repo.append_messages(run.session_id, &new_msgs)
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
            1,
            Some(&json!({ "step_index": step_index })),
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}
