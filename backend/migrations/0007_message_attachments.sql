ALTER TABLE session_messages
    ADD COLUMN attachments TEXT NOT NULL DEFAULT '[]';
ALTER TABLE session_messages
    ADD COLUMN artifacts TEXT NOT NULL DEFAULT '[]';
