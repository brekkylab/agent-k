//! Automation worker loop. Claims queued runs and drives a **single prompt**
//! against the run's session to completion.
//!
//! Crash safety: each claimed run has its `lease_until` heartbeated by a
//! companion task. If a heartbeat finds the row no longer ours (a reaper
//! requeued it) it cancels the in-flight agent. A housekeeper periodically
//! requeues expired-lease rows. A cron ticker fires due cron triggers.
//!
//! Single-prompt note: there is no per-step cursor, so a re-claimed run simply
//! re-runs its one prompt on the (message-less) session — no replay/orphan.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ailoy::message::Part;
use chrono::Utc;
use futures_util::StreamExt as _;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use workspace::WorkspaceFs;

use crate::{
    cron::{default_tz_name, next_fire_after},
    state::{
        AppState, AutomationRun, AutomationTrigger, Change, Condition, RunLogKind, MAX_ATTEMPTS,
        MAX_CHAIN_DEPTH, NewEvent, RunStatus, Snapshot, TriggerSpec, diff_snapshots,
    },
};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const REAP_INTERVAL: Duration = Duration::from_secs(60);
const CRON_TICK_INTERVAL: Duration = Duration::from_secs(15);
const DISPATCH_INTERVAL: Duration = Duration::from_secs(2);
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
    spawn_cron_ticker(state.clone());
    spawn_dispatcher(state.clone());
    spawn_source_poller(state);
    tracing::info!(workers = count, "automation runtime spawned");
}

/// Interval between external-source poll sweeps. `0` disables polling.
fn source_poll_interval() -> Duration {
    let secs = std::env::var("AGENT_K_SOURCE_POLL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60);
    Duration::from_secs(secs)
}

/// Cap on files recorded per mount per poll — a backstop against walking an
/// enormous provider tree.
const SCAN_ENTRY_CAP: usize = 5000;

/// Periodically poll subscribed external mounts and emit change events.
fn spawn_source_poller(state: Arc<AppState>) {
    let interval = source_poll_interval();
    if interval.is_zero() {
        tracing::info!("source polling disabled (AGENT_K_SOURCE_POLL_SECS=0)");
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(e) = poll_sources_once(&state).await {
                tracing::error!("source poll failed: {e}");
            }
        }
    });
}

/// The provider a subscribed event `source` belongs to (`s3.*` → `"s3"`), or
/// `None` for non-provider sources (workspace/knowledge/run/webhook).
fn provider_of_source(source: &str) -> Option<&'static str> {
    if source.starts_with("s3.") {
        Some("s3")
    } else if source.starts_with("notion.") {
        Some("notion")
    } else {
        None
    }
}

/// The event kind for a provider + change (e.g. s3 + Created → s3.object_created).
fn source_event_kind(provider: &str, change: &Change) -> &'static str {
    match (provider, change) {
        ("s3", Change::Created(_)) => "s3.object_created",
        ("s3", Change::Modified(_)) => "s3.object_modified",
        ("s3", Change::Removed(_)) => "s3.object_removed",
        ("notion", Change::Created(_)) => "notion.page_created",
        ("notion", Change::Modified(_)) => "notion.page_updated",
        ("notion", Change::Removed(_)) => "notion.page_removed",
        // Unknown provider: a generic, still-filterable fallback.
        (_, Change::Created(_)) => "source.created",
        (_, Change::Modified(_)) => "source.modified",
        (_, Change::Removed(_)) => "source.removed",
    }
}

/// One poll sweep: for each workspace with an enabled event subscriber, poll its
/// mounts of the subscribed providers (lazy activation), diff, and emit events.
async fn poll_sources_once(state: &Arc<AppState>) -> Result<(), String> {
    let subs = state
        .automations
        .list_enabled_event_trigger_sources()
        .await
        .map_err(|e| e.to_string())?;
    // workspace_id → set of subscribed providers.
    let mut want: HashMap<Uuid, HashSet<&'static str>> = HashMap::new();
    for (wid, sources) in subs {
        for s in &sources {
            if let Some(p) = provider_of_source(s) {
                want.entry(wid).or_default().insert(p);
            }
        }
    }
    for (wid, providers) in want {
        let mounts = match state.workspaces.list_mounts(wid).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(%wid, "poll: list_mounts failed: {e}");
                continue;
            }
        };
        for m in mounts {
            let provider = m.provider_str();
            if !providers.contains(provider) {
                continue;
            }
            if let Err(e) = poll_mount(state, wid, &m.prefix, provider).await {
                tracing::warn!(%wid, prefix = %m.prefix, "poll mount failed: {e}");
            }
        }
    }
    Ok(())
}

/// Scan one mount, diff against its stored snapshot, emit change events, and save
/// the new snapshot. The first poll only seeds the baseline (emits nothing).
async fn poll_mount(
    state: &Arc<AppState>,
    wid: Uuid,
    prefix: &str,
    provider: &str,
) -> Result<(), String> {
    let fs = state.workspaces.scan_fs(wid).await.map_err(|e| e.to_string())?;
    let mut next = Snapshot::new();
    scan_tree(&fs, prefix, &mut next, 0).await;
    if next.len() >= SCAN_ENTRY_CAP {
        // The scan was truncated: the partial snapshot is order-dependent, so
        // diffs would churn. Skip emitting/persisting rather than fire spurious
        // events for a mount this large.
        tracing::warn!(%wid, prefix, cap = SCAN_ENTRY_CAP, "source poll: mount exceeds scan cap — skipping");
        return Ok(());
    }

    match state.source_poll.get(wid, prefix).await.map_err(|e| e.to_string())? {
        None => {} // first poll — seed baseline below without emitting
        Some(prev) => {
            for change in diff_snapshots(&prev, &next) {
                let path = match &change {
                    Change::Created(p) | Change::Modified(p) | Change::Removed(p) => p.clone(),
                };
                let kind = source_event_kind(provider, &change);
                if let Err(e) = state
                    .event_store
                    .emit(NewEvent {
                        kind: kind.into(),
                        workspace_id: Some(wid),
                        payload: Some(json!({
                            "workspace_id": wid.to_string(),
                            "prefix": prefix,
                            "path": path,
                        })),
                    })
                    .await
                {
                    tracing::error!(%wid, "emit {kind} failed: {e}");
                }
            }
        }
    }
    state.source_poll.put(wid, prefix, &next).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Recursively walk `path` under a mount, recording each file's change signature
/// (`modified:len`) into `out`. Bounded by [`SCAN_ENTRY_CAP`] and a depth cap.
async fn scan_tree(fs: &WorkspaceFs, path: &str, out: &mut Snapshot, depth: usize) {
    if out.len() >= SCAN_ENTRY_CAP || depth > 32 {
        return;
    }
    let Ok(mut stream) = fs.read_dir(path).await else {
        return;
    };
    while let Some(entry) = stream.next().await {
        let Ok(entry) = entry else { continue };
        let name = String::from_utf8_lossy(&entry.name()).into_owned();
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        let child = format!("{}/{}", path.trim_end_matches('/'), name);
        match entry.metadata() {
            Ok(stat) if stat.is_dir() => {
                Box::pin(scan_tree(fs, &child, out, depth + 1)).await;
            }
            Ok(stat) => {
                // Stable signature (`nanos:len`): a `Debug` format of SystemTime
                // could change across a library update and re-fire every file.
                let mtime_ns = stat
                    .modified
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                out.insert(child, format!("{mtime_ns}:{}", stat.len));
            }
            Err(_) => {}
        }
        if out.len() >= SCAN_ENTRY_CAP {
            break;
        }
    }
}

/// Poll the durable event outbox and fire matching event triggers.
fn spawn_dispatcher(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(DISPATCH_INTERVAL).await;
            if let Err(e) = dispatch_once(&state).await {
                tracing::error!("event dispatch failed: {e}");
            }
        }
    });
}

async fn dispatch_once(state: &Arc<AppState>) -> Result<(), String> {
    let events = state
        .event_store
        .list_undispatched(100)
        .await
        .map_err(|e| e.to_string())?;
    if events.is_empty() {
        return Ok(());
    }
    // Pre-parse enabled event triggers into (trigger, conditions).
    let triggers: Vec<(AutomationTrigger, Vec<Condition>)> = state
        .automations
        .list_event_triggers()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|t| match TriggerSpec::from_db(t.kind, &t.spec_json) {
            Ok(TriggerSpec::Event { conditions }) => Some((t, conditions)),
            _ => None,
        })
        .collect();

    for event in events {
        // Causal chain depth of any run this event spawns: a `run.succeeded`
        // event continues its producer's chain (parent + 1); every other source
        // roots a fresh chain at 0. Computed once per event (same for all
        // matching triggers).
        let spawn_depth = if event.kind == "run.succeeded" {
            let parent = event
                .payload
                .as_ref()
                .and_then(|p| p.get("run_id"))
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let parent_depth = match parent {
                Some(rid) => state
                    .automations
                    .run_chain_depth(rid)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(0),
                None => 0,
            };
            parent_depth + 1
        } else {
            0
        };

        // A transient DB error on any trigger leaves the event undispatched so the
        // next sweep retries it; the (trigger, event) dedup makes reprocessing of
        // already-created runs a no-op.
        let mut all_ok = true;
        for (trigger, conditions) in &triggers {
            // OR: fire if any condition (its own source + filter) matches.
            if !conditions.iter().any(|c| c.matches(&event.kind, event.payload.as_ref())) {
                continue;
            }
            // Loop guard: suppress the run once the causal chain hits the cap.
            if spawn_depth > MAX_CHAIN_DEPTH {
                tracing::warn!(
                    trigger = %trigger.id, event = %event.id, depth = spawn_depth,
                    "chain depth cap ({MAX_CHAIN_DEPTH}) reached — run suppressed"
                );
                continue;
            }
            // Idempotency: one run per (trigger, event) — guards re-processing.
            match state.automations.run_exists_for_event(trigger.id, event.id).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => {
                    tracing::error!(trigger = %trigger.id, "run_exists check failed: {e}");
                    all_ok = false;
                    continue;
                }
            }
            let automation = match state.automations.get_automation(trigger.automation_id).await {
                Ok(Some(a)) => a,
                Ok(None) => continue,
                Err(e) => {
                    tracing::error!(trigger = %trigger.id, "get_automation failed: {e}");
                    all_ok = false;
                    continue;
                }
            };
            let input = event.payload.clone().unwrap_or_else(|| json!({}));
            match state
                .automations
                .create_event_run(&automation, trigger.id, event.id, input, spawn_depth)
                .await
            {
                Ok(Some(run)) => {
                    tracing::info!(trigger = %trigger.id, run = %run.id, event = %event.id, "event trigger fired")
                }
                Ok(None) => {} // automation disabled
                Err(e) => {
                    tracing::error!(trigger = %trigger.id, "create_event_run failed: {e}");
                    all_ok = false;
                }
            }
        }
        // Only consume the event once every matching trigger was handled cleanly.
        if all_ok {
            state
                .event_store
                .mark_dispatched(event.id)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
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
        return Err("non-cron trigger surfaced in cron path".into());
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
        Ok(_) => {
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

    // Chaining: a successful run emits `run.succeeded` carrying its output, which
    // other automations can subscribe to (fan-out via multiple subscribers).
    if finalize_owned && matches!(final_status, RunStatus::Succeeded) {
        let output = result.ok().flatten();
        let payload = json!({
            "automation_id": run.automation_id.map(|a| a.to_string()),
            "run_id": run.id.to_string(),
            "output": output,
        });
        if let Err(e) = state
            .event_store
            .emit(crate::state::NewEvent {
                kind: "run.succeeded".into(),
                workspace_id: Some(run.workspace_id),
                payload: Some(payload),
            })
            .await
        {
            tracing::error!(run = %run.id, "emit run.succeeded failed: {e}");
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
) -> Result<Option<String>, String> {
    state
        .automations
        .append_log(run.id, RunLogKind::Started, None)
        .await
        .map_err(|e| e.to_string())?;

    // Render the prompt template at execution time against the triggering event
    // payload (`{{event.*}}`) + date builtins. Fail-closed on undefined vars.
    let prompt = crate::state::render(&run.prompt, run.input.as_ref())
        .map_err(|e| format!("template render: {e}"))?;
    let parts = vec![Part::text(prompt)];
    state
        .sessions
        .drive_prompt(run.session_id, parts, cancel.clone())
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::dispatch_once;
    use crate::auth::JwtConfig;
    use crate::state::{AppState, Condition, MAX_CHAIN_DEPTH, NewEvent, TriggerSpec, render};
    use chrono::Utc;
    use serde_json::json;
    use sqlx::SqlitePool;
    use std::sync::Arc;
    use uuid::Uuid;

    /// Fresh AppState on a temp file DB, plus a second pool to the same file for
    /// seeding/asserting (AppState owns its pool privately). Returns
    /// `(state, seed_pool, dir)`; caller removes `dir`.
    async fn boot() -> (Arc<AppState>, SqlitePool, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("agentk-e2e-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_url = format!("sqlite://{}/e2e.db", dir.display());
        let state = AppState::new(&db_url, dir.clone(), JwtConfig::new("test", 3600))
            .await
            .unwrap();
        let pool = SqlitePool::connect(&db_url).await.unwrap();
        (Arc::new(state), pool, dir)
    }

    async fn seed_user_ws(pool: &SqlitePool) -> (Uuid, Uuid) {
        let (uid, wid) = (Uuid::new_v4(), Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, is_active, preferred_language, created_at, updated_at) \
             VALUES (?, ?, 'x', 'user', 1, 'en', ?, ?)",
        )
        .bind(uid.to_string()).bind(format!("u-{uid}")).bind(&now).bind(&now)
        .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, user_id, title, created_at, updated_at) VALUES (?, ?, 'W', ?, ?)")
            .bind(wid.to_string()).bind(uid.to_string()).bind(&now).bind(&now)
            .execute(pool).await.unwrap();
        (uid, wid)
    }

    /// End-to-end through the real dispatch pipeline (no agent execution):
    /// outbox event → `dispatch_once` → run creation, covering filter matching,
    /// payload→input+render, dedup, `run.succeeded` chaining, and the
    /// chain-depth loop guard.
    #[tokio::test]
    async fn dispatch_pipeline_e2e() {
        let (state, pool, dir) = boot().await;
        let (uid, wid) = seed_user_ws(&pool).await;

        // Automation A: knowledge.indexed for *.pdf; prompt renders {{event.path}}.
        let a = state
            .automations
            .create_automation(
                wid, "index pdf".into(), None,
                "New PDF at {{event.path}}".into(), "coworker".into(), None, uid,
            )
            .await.unwrap();
        let a_spec = TriggerSpec::Event {
            conditions: vec![Condition {
                source: "knowledge.indexed".into(),
                filter: Some(json!({ "path": { "glob": "*.pdf" } })),
            }],
        };
        state.automations.create_trigger(a.id, &a_spec, true, None, None).await.unwrap();

        // (1) Non-matching event (.txt) → no run.
        state.event_store.emit(NewEvent {
            kind: "knowledge.indexed".into(),
            workspace_id: Some(wid),
            payload: Some(json!({ "path": "/knowledge/note.txt" })),
        }).await.unwrap();
        dispatch_once(&state).await.unwrap();
        assert_eq!(state.automations.list_runs(a.id).await.unwrap().len(), 0, "txt must not match *.pdf");

        // (2) Matching event (.pdf) → one run; input = payload, event set, depth 0.
        let ev = state.event_store.emit(NewEvent {
            kind: "knowledge.indexed".into(),
            workspace_id: Some(wid),
            payload: Some(json!({ "path": "/knowledge/report.pdf" })),
        }).await.unwrap();
        dispatch_once(&state).await.unwrap();
        let runs = state.automations.list_runs(a.id).await.unwrap();
        assert_eq!(runs.len(), 1, "pdf must trigger exactly one run");
        let a_run = runs[0].clone();
        assert_eq!(a_run.event_id, Some(ev.id));
        assert_eq!(a_run.chain_depth, 0);
        assert_eq!(a_run.input.as_ref().unwrap()["path"], "/knowledge/report.pdf");
        assert_eq!(
            render(&a_run.prompt, a_run.input.as_ref()).unwrap(),
            "New PDF at /knowledge/report.pdf",
        );

        // (3) Dedup: the event is marked dispatched; re-dispatch adds nothing.
        dispatch_once(&state).await.unwrap();
        assert_eq!(state.automations.list_runs(a.id).await.unwrap().len(), 1, "no duplicate run");

        // (4) Chaining: B subscribes to run.succeeded; A's success spawns a B run
        //     at parent depth + 1.
        let b = state
            .automations
            .create_automation(wid, "on success".into(), None, "chained".into(), "coworker".into(), None, uid)
            .await.unwrap();
        let b_spec = TriggerSpec::Event {
            conditions: vec![Condition { source: "run.succeeded".into(), filter: None }],
        };
        state.automations.create_trigger(b.id, &b_spec, true, None, None).await.unwrap();

        state.event_store.emit(NewEvent {
            kind: "run.succeeded".into(),
            workspace_id: Some(wid),
            payload: Some(json!({ "automation_id": a.id.to_string(), "run_id": a_run.id.to_string(), "output": "done" })),
        }).await.unwrap();
        dispatch_once(&state).await.unwrap();
        let b_runs = state.automations.list_runs(b.id).await.unwrap();
        assert_eq!(b_runs.len(), 1, "run.succeeded must chain into B");
        assert_eq!(b_runs[0].chain_depth, 1, "chained run is parent depth + 1");

        // (5) Loop guard: a run.succeeded from a run already AT the cap is
        //     suppressed (spawn depth would exceed MAX_CHAIN_DEPTH).
        sqlx::query("UPDATE automation_runs SET chain_depth = ? WHERE id = ?")
            .bind(MAX_CHAIN_DEPTH).bind(b_runs[0].id.to_string())
            .execute(&pool).await.unwrap();
        state.event_store.emit(NewEvent {
            kind: "run.succeeded".into(),
            workspace_id: Some(wid),
            payload: Some(json!({ "run_id": b_runs[0].id.to_string() })),
        }).await.unwrap();
        dispatch_once(&state).await.unwrap();
        assert_eq!(
            state.automations.list_runs(b.id).await.unwrap().len(),
            1,
            "chain at cap must be suppressed",
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
