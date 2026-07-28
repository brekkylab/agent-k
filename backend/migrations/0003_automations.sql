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

-- Triggers. Cron: `spec_json` holds {expr, tz}, `next_fire_at` the next UTC
-- instant (NULL when disabled). Event: `spec_json` holds the OR-combined
-- conditions ([{source, filter}]); the dispatcher matches events to these.
CREATE TABLE automation_triggers (
    id            TEXT PRIMARY KEY,
    automation_id TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL,
    spec_json     TEXT NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    next_fire_at  TEXT,
    webhook_token_hash TEXT,                     -- webhook only; sha256 hex of the issued token
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
-- A webhook token identifies its trigger globally; the hash is the lookup key.
CREATE UNIQUE INDEX idx_triggers_webhook_token
    ON automation_triggers(webhook_token_hash)
    WHERE webhook_token_hash IS NOT NULL;
CREATE INDEX idx_triggers_automation ON automation_triggers(automation_id);
-- Partial index for the cron ticker's "due" scan.
CREATE INDEX idx_triggers_due ON automation_triggers(next_fire_at)
    WHERE enabled = 1 AND next_fire_at IS NOT NULL;

-- Durable event outbox. Producers append domain events (run.succeeded,
-- workspace.created, knowledge.indexed, …); the dispatcher scans undispatched
-- rows, matches them to event triggers (source + filter), and fires runs.
-- Distinct from the in-process, ephemeral EventQueue (event.rs), which only
-- powers live WS streaming. Declared before automation_runs so the run's
-- event_id FK below resolves.
CREATE TABLE events (
    id             TEXT PRIMARY KEY,
    type           TEXT NOT NULL,
    workspace_id   TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
    payload        TEXT,
    created_at     TEXT NOT NULL,
    dispatched_at  TEXT
);
CREATE INDEX idx_events_undispatched ON events(created_at) WHERE dispatched_at IS NULL;
CREATE INDEX idx_events_type ON events(type, workspace_id);

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
    -- Event-triggered runs carry the triggering event's payload as input
    -- (rendered into the prompt template) and record the event that fired them.
    input           TEXT,
    event_id        TEXT REFERENCES events(id) ON DELETE SET NULL,
    -- Causal chain depth: 0 for roots (cron/ad-hoc/non-run events), parent+1
    -- for runs triggered by a `run.succeeded` event. Capped at dispatch to break
    -- run→run trigger loops (see worker.rs MAX_CHAIN_DEPTH).
    chain_depth     INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_runs_workspace ON automation_runs(workspace_id, created_at DESC);
CREATE INDEX idx_runs_automation_created ON automation_runs(automation_id, created_at DESC);
CREATE INDEX idx_runs_status_scheduled   ON automation_runs(status, scheduled_for);
CREATE INDEX idx_runs_lease              ON automation_runs(lease_until)
    WHERE status = 'running';
-- One run per (trigger, event): guards against duplicate runs if an event is
-- re-processed after a crash mid-dispatch.
CREATE UNIQUE INDEX idx_runs_trigger_event
    ON automation_runs(trigger_id, event_id)
    WHERE trigger_id IS NOT NULL AND event_id IS NOT NULL;

-- Append-only run log (lifecycle entries: triggered/queued/started/…). Distinct
-- from the `events` outbox above — this is the per-run audit trail, that is the
-- trigger event bus. Full lifecycle set minus step_started/step_finished
-- (single-prompt runs have no per-step cursor).
CREATE TABLE automation_run_logs (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id   TEXT NOT NULL REFERENCES automation_runs(id) ON DELETE CASCADE,
    ts       TEXT NOT NULL,
    kind     TEXT NOT NULL,
    payload  TEXT
);
CREATE INDEX idx_run_logs_run_ts ON automation_run_logs(run_id, ts);
