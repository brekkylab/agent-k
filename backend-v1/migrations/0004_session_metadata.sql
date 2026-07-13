ALTER TABLE sessions ADD COLUMN title TEXT;
ALTER TABLE sessions ADD COLUMN last_message_at TEXT;
ALTER TABLE sessions ADD COLUMN last_message_snippet TEXT;

CREATE TABLE session_reads (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    last_read_seq INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (session_id, user_id)
);

CREATE INDEX idx_session_reads_user ON session_reads(user_id);

-- Backfill last_message_at for any existing sessions
UPDATE sessions
SET last_message_at = (
    SELECT MAX(created_at) FROM session_messages WHERE session_id = sessions.id
);
