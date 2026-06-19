-- Team messages: user-to-user messages ('@' mentions) that are stored and
-- listed but never delivered to the agent.
ALTER TABLE session_messages
    ADD COLUMN message_kind TEXT NOT NULL DEFAULT 'chat'
    CHECK (message_kind IN ('chat', 'team'));

-- Mentioned user UUIDs as a JSON string array (same pattern as attachments).
ALTER TABLE session_messages
    ADD COLUMN mentions TEXT NOT NULL DEFAULT '[]';
