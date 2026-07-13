DELETE FROM session_messages;
UPDATE sessions
   SET last_message_at = NULL,
       last_message_snippet = NULL;
DELETE FROM session_reads;

ALTER TABLE session_messages
    ADD COLUMN sender_kind TEXT NOT NULL DEFAULT 'agent'
    CHECK (sender_kind IN ('user', 'agent'));
ALTER TABLE session_messages
    ADD COLUMN sender_name TEXT;
ALTER TABLE session_messages
    ADD COLUMN sender_user_id TEXT;
