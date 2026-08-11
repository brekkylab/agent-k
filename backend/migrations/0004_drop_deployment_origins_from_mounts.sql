-- no-transaction
--
-- Strip the deployment's service endpoints out of Gmail and Drive mount rows, and scrub
-- what the previous migration left behind in the file.
--
-- `origins` (where each Google service is reached) belongs to the installation, not to
-- any one mount: it was written from the backend's own config at mount-create, identical
-- in every row. Unlike the client pair that `0003` removed it is not a secret, but it
-- carries the OAuth token endpoint, so while it was read from the row, anyone able to
-- write one named the host that `client_id`, `client_secret` and `refresh_token` were
-- POSTed to. The backend now supplies it when it builds a mount, and nothing reads the
-- stored copy.
--
-- The three keys are removed together so a row written by any earlier version converges,
-- whether or not `0003` already ran. Same `json_remove` treatment: every other field
-- (refresh_token, account_email, index_cap) is preserved exactly. The
-- `json_extract(...) IS NOT NULL` clause is load-bearing beyond selecting rows that need
-- work, because `json_valid` alone would admit a valid non-object config that
-- `json_remove` would rewrite.
--
-- ROTATION IS STILL REQUIRED, and the VACUUM does not replace it. An UPDATE rewrites the
-- live cell and leaves the previous bytes behind; this pool sets neither `secure_delete`
-- nor `auto_vacuum`. Measured with `strings` on a WAL database of 20 rows carrying the
-- same secret, counting the main file and the `-wal` separately:
--
--     after a checkpoint, before any strip     db 20/20     wal has copies
--     0003 applied, connection open            db 20/20     wal has copies
--     0003 applied, then checkpointed          db 10/20     wal has copies
--     this migration's VACUUM, before ckpt     db 10/20     wal has copies
--     VACUUM, then checkpointed                db  0        wal has copies
--     clean shutdown (the -wal is removed)     db  0        -
--
-- So the VACUUM does reach zero, but only in the main file and only once the WAL has been
-- checkpointed: a crash before that leaves the `-wal` holding the secret. And nothing here
-- reaches filesystem free blocks, pages an SSD has remapped, or a backup taken earlier. On
-- any database that ran the pre-0003 code the client secret must be treated as disclosed
-- and the Google client rotated; this migration narrows the window, it does not close it.
--
-- `-- no-transaction` is what lets VACUUM run at all: sqlx wraps a migration in a
-- transaction by default, and VACUUM cannot execute inside one. The trade is that this
-- migration has no rollback, which is acceptable because the UPDATE is idempotent, so a
-- partial application converges on the next run.
UPDATE workspace_mounts
SET config = json_remove(config, '$.client_id', '$.client_secret', '$.origins')
WHERE provider IN ('gmail', 'gdrive')
  AND json_valid(config)
  AND (
      json_extract(config, '$.client_id') IS NOT NULL
      OR json_extract(config, '$.client_secret') IS NOT NULL
      OR json_extract(config, '$.origins') IS NOT NULL
  );

VACUUM;
