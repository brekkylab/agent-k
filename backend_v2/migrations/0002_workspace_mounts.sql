-- External-provider mounts attached to a workspace. Each row binds a virtual
-- top-level prefix (e.g. `/s3-prod`) to a provider config. Credentials are
-- stored in `config` as JSON, in plaintext (matching the repo's handling of
-- other secrets); revisit with an encryption layer if the threat model changes.
CREATE TABLE workspace_mounts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    prefix TEXT NOT NULL,
    provider TEXT NOT NULL,
    config TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_workspace_mounts_workspace ON workspace_mounts(workspace_id);
CREATE UNIQUE INDEX idx_workspace_mounts_prefix ON workspace_mounts(workspace_id, prefix);
