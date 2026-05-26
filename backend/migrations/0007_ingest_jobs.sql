-- Auto-ingest pipeline: enqueue dirent uploads (PDF/MD/TXT) for Store ingestion
-- by a background worker. project_documents tracks the resulting (project, path)
-- → document_id mapping so that delete and re-ingest are idempotent.

CREATE TABLE ingest_jobs (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_path     TEXT NOT NULL,            -- relative path under projects/{id}/uploads/
    status          TEXT NOT NULL,            -- queued | running | done | failed
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    lease_until     TEXT,                     -- NULL when not leased; reaper uses this
    document_id     TEXT,                     -- agent-k Store document id once status=done
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX idx_ingest_jobs_status_created ON ingest_jobs(status, created_at);
CREATE INDEX idx_ingest_jobs_project ON ingest_jobs(project_id);
CREATE INDEX idx_ingest_jobs_lease ON ingest_jobs(lease_until) WHERE status = 'running';
-- One queued/running/done row per (project, path). On re-upload we update the
-- existing row (status -> queued, attempts reset) rather than insert a new one.
CREATE UNIQUE INDEX idx_ingest_jobs_project_path ON ingest_jobs(project_id, source_path);

-- Mapping from (project, source_path) to the Store document. Populated when an
-- ingest_job completes; deleted when dirent.delete removes the file.
CREATE TABLE project_documents (
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_path     TEXT NOT NULL,
    document_id     TEXT NOT NULL,
    ingested_at     TEXT NOT NULL,
    PRIMARY KEY (project_id, source_path)
);

CREATE INDEX idx_project_documents_doc ON project_documents(document_id);
