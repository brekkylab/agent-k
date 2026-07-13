-- Add slug column to projects and a history table for retired slugs.
-- Forward-only migration. Existing rows get an empty slug temporarily; a
-- one-time backfill is required if any project rows exist (dev policy is to
-- recreate backend/data/app.db, so the DEFAULT '' will never persist).

ALTER TABLE projects ADD COLUMN slug TEXT NOT NULL DEFAULT '';

CREATE UNIQUE INDEX idx_projects_slug ON projects(slug);

CREATE TABLE project_slug_history (
    old_slug   TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    retired_at TEXT NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX idx_project_slug_history_project ON project_slug_history(project_id);
