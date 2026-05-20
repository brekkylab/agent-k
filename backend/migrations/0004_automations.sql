-- Agent automation: definitions, triggers, run instances, run events.
-- sessions.origin tags user-created vs automation-created sessions.

ALTER TABLE sessions ADD COLUMN origin TEXT NOT NULL DEFAULT 'user';
CREATE INDEX idx_sessions_project_origin ON sessions(project_id, origin, created_at);

CREATE TABLE automations (
    id                   TEXT PRIMARY KEY,
    project_id           TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name                 TEXT NOT NULL,
    description          TEXT,

    -- Ordered list of prompt templates run sequentially in the same session.
    -- JSON array of strings; future per-step metadata can migrate to objects.
    prompts_json         TEXT NOT NULL DEFAULT '[]',

    created_by           TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);
CREATE INDEX idx_automations_project ON automations(project_id);

-- kind ∈ {'cron','webhook'}, validated in code.
-- spec_json: cron -> {"expr","tz"}; webhook -> {} (no fields in v1).
CREATE TABLE automation_triggers (
    id                   TEXT PRIMARY KEY,
    automation_id        TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    kind                 TEXT NOT NULL,
    spec_json            TEXT NOT NULL DEFAULT '{}',
    enabled              INTEGER NOT NULL DEFAULT 1,
    next_fire_at         TEXT,                       -- cron only
    webhook_token_hash   TEXT,                       -- webhook only; sha256 hex of issued token
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);
CREATE INDEX idx_triggers_automation ON automation_triggers(automation_id);
CREATE INDEX idx_triggers_cron_due   ON automation_triggers(enabled, next_fire_at)
    WHERE kind = 'cron';
CREATE UNIQUE INDEX idx_triggers_webhook_token
    ON automation_triggers(webhook_token_hash)
    WHERE webhook_token_hash IS NOT NULL;

-- 1 run == 1 session. trigger_id=NULL for ad-hoc/manual runs.
-- started_at / finished_at / failure reason live on
-- automation_run_events; status is kept here for cheap list queries.
CREATE TABLE automation_runs (
    id                   TEXT PRIMARY KEY,
    automation_id        TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    trigger_id           TEXT REFERENCES automation_triggers(id) ON DELETE SET NULL,
    session_id           TEXT NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    status               TEXT NOT NULL,              -- queued|running|succeeded|failed|cancelled|timeout
    scheduled_for        TEXT NOT NULL,              -- earliest pickup time
    lease_until          TEXT,                       -- NULL when not leased; reaper uses this
    previous_run_id      TEXT REFERENCES automation_runs(id) ON DELETE SET NULL,
    -- Caller-provided idempotency key for webhook-triggered runs. NULL for
    -- manual / cron / non-keyed webhook fires.
    idempotency_key      TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);
CREATE INDEX idx_runs_automation_created ON automation_runs(automation_id, created_at DESC);
CREATE INDEX idx_runs_trigger            ON automation_runs(trigger_id);
CREATE INDEX idx_runs_status_scheduled   ON automation_runs(status, scheduled_for);
CREATE INDEX idx_runs_lease              ON automation_runs(lease_until)
    WHERE status = 'running';
-- Used for webhook idempotency lookups + acts as the final guard against
-- concurrent webhook retries inserting duplicate runs.
CREATE UNIQUE INDEX idx_runs_trigger_idempotency
    ON automation_runs(trigger_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- Append-only audit log. `ts` is the event time AND row insertion time, so no
-- separate created_at. Conversation turns stay in session_messages; this table
-- holds state transitions and operational meta-events.
CREATE TABLE automation_run_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id      TEXT NOT NULL REFERENCES automation_runs(id) ON DELETE CASCADE,
    ts          TEXT NOT NULL,
    kind        TEXT NOT NULL,
    payload     TEXT
);
CREATE INDEX idx_run_events_run_ts ON automation_run_events(run_id, ts);
CREATE INDEX idx_run_events_kind   ON automation_run_events(kind, ts);
