-- migrations/0003_projects_and_sessions.sql
DROP TABLE IF EXISTS session_messages;
DROP TABLE IF EXISTS sessions;

CREATE TABLE projects (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    description  TEXT,
    owner_id     TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX idx_projects_owner ON projects(owner_id);

CREATE TABLE project_members (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    added_at   TEXT NOT NULL,
    PRIMARY KEY (project_id, user_id)
);
CREATE INDEX idx_project_members_user ON project_members(user_id);

CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    creator_id  TEXT NOT NULL REFERENCES users(id)    ON DELETE RESTRICT,
    share_mode  TEXT NOT NULL DEFAULT 'private'
                CHECK (share_mode IN ('private', 'shared_readonly', 'shared_chat')),
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_sessions_project ON sessions(project_id);
CREATE INDEX idx_sessions_creator ON sessions(creator_id);

CREATE TABLE session_messages (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_json TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_session_messages_session_seq ON session_messages(session_id, seq);
