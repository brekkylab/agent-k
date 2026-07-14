-- Automations: a named, workspace-scoped agent job. Single prompt (v1's
-- multi-step `prompts` array is intentionally collapsed to one prompt here —
-- there is no cross-step progress to track, so a re-claimed run just re-runs
-- the one prompt on a fresh session).
CREATE TABLE automations (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    description  TEXT,
    prompt       TEXT NOT NULL,
    -- Agent surface for triggered runs: preset name + optional model pin. Built
    -- into an AgentSpec at run time (see state/automation.rs `build_spec`).
    agent_type   TEXT NOT NULL DEFAULT 'coworker',
    model        TEXT,
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_by   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX idx_automations_workspace ON automations(workspace_id, created_at DESC);

-- Cron triggers (webhook out of scope). `spec_json` holds the untagged variant
-- fields ({expr, tz}); `next_fire_at` is the next scheduled UTC instant, NULL
-- when the trigger is disabled.
CREATE TABLE automation_triggers (
    id            TEXT PRIMARY KEY,
    automation_id TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL,
    spec_json     TEXT NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    next_fire_at  TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_triggers_automation ON automation_triggers(automation_id);
-- Partial index for the cron ticker's "due" scan.
CREATE INDEX idx_triggers_due ON automation_triggers(next_fire_at)
    WHERE enabled = 1 AND next_fire_at IS NOT NULL;

-- Runs: one session per run. Lease/reaper columns support crash-safe execution.
-- A run is self-describing: it snapshots the rendered prompt + agent surface it
-- executes, so it depends on no automation at run time and its audit trail
-- survives later edits/deletion of the automation.
CREATE TABLE automation_runs (
    id              TEXT PRIMARY KEY,
    -- NULL for ad-hoc one-time runs (they depend on no automation). ON DELETE
    -- SET NULL so deleting an automation preserves its runs as an audit record.
    automation_id   TEXT REFERENCES automations(id) ON DELETE SET NULL,
    trigger_id      TEXT REFERENCES automation_triggers(id) ON DELETE SET NULL,
    session_id      TEXT NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- Rendered prompt actually executed + agent surface, snapshotted at creation.
    prompt          TEXT NOT NULL,
    agent_type      TEXT NOT NULL DEFAULT 'coworker',
    model           TEXT,
    status          TEXT NOT NULL,            -- queued | running | succeeded | failed | cancelled
    scheduled_for   TEXT NOT NULL,            -- earliest pickup time
    lease_until     TEXT,                     -- NULL when not leased; reaper uses this
    previous_run_id TEXT REFERENCES automation_runs(id) ON DELETE SET NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_runs_workspace ON automation_runs(workspace_id, created_at DESC);
CREATE INDEX idx_runs_automation_created ON automation_runs(automation_id, created_at DESC);
CREATE INDEX idx_runs_status_scheduled   ON automation_runs(status, scheduled_for);
CREATE INDEX idx_runs_lease              ON automation_runs(lease_until)
    WHERE status = 'running';

-- Append-only run event log. `ts` is both event time and row insertion time.
-- Full lifecycle set minus step_started/step_finished (single-prompt runs have
-- no per-step cursor).
CREATE TABLE automation_run_logs (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id   TEXT NOT NULL REFERENCES automation_runs(id) ON DELETE CASCADE,
    ts       TEXT NOT NULL,
    kind     TEXT NOT NULL,
    payload  TEXT
);
CREATE INDEX idx_run_logs_run_ts ON automation_run_logs(run_id, ts);
