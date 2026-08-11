-- Strip the deployment's own configuration out of Gmail and Drive mount rows.
--
-- Three fields, all belonging to the installation rather than to any one mount, and all
-- written from the backend's own config at mount-create time: `client_id`,
-- `client_secret`, and `origins` (where each Google service is reached). The backend now
-- supplies them when it builds a mount, so the stored copies are read by nothing.
--
-- They mattered in different ways. The client pair is needed on every refresh, an access
-- token lasting an hour, and sitting beside a refresh token it made one leaked row a
-- working Google credential; a refresh token alone mints nothing. `origins` is not a
-- secret but carries the token endpoint, so a row that could be *written* named the host
-- that pair was POSTed to.
--
-- IMPORTANT, and not achieved by this migration: on any database that ran the earlier
-- code, the client secret must be treated as disclosed and the Google client rotated. An
-- UPDATE rewrites the live cell; it does not scrub the previous bytes from the file, and
-- this pool sets neither `secure_delete` nor `auto_vacuum`. Measured on a WAL database of
-- 20 rows, 12 of the stripped secrets were still recoverable with `strings` afterwards,
-- and every backup taken before the migration keeps them regardless. VACUUM helps but is
-- not dependable, and cannot run here in any case: sqlx wraps each migration in a
-- transaction.
--
-- `config` is a JSON object, so the keys are removed with json_remove rather than by
-- rewriting the document; every other field (refresh_token, account_email, index_cap) is
-- preserved exactly. The `json_extract(...) IS NOT NULL` clause is load-bearing beyond
-- selecting rows that need work: `json_valid` alone would admit a valid non-object
-- config, which json_remove would happily rewrite. Rows already written without the keys
-- are untouched, which makes this safe on a database that saw both code versions.
UPDATE workspace_mounts
SET config = json_remove(config, '$.client_id', '$.client_secret', '$.origins')
WHERE provider IN ('gmail', 'gdrive')
  AND json_valid(config)
  AND (
      json_extract(config, '$.client_id') IS NOT NULL
      OR json_extract(config, '$.client_secret') IS NOT NULL
      OR json_extract(config, '$.origins') IS NOT NULL
  );
