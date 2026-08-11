-- Strip the deployment's Google OAuth client out of Gmail and Drive mount rows.
--
-- Both halves are genuinely needed at runtime (a Google access token lasts an hour,
-- and the refresh grant is rejected without them), but they belong to the installation
-- rather than to any one mount: the backend already holds them in its own config and
-- now injects them when it builds a mount. A stored copy was one credential duplicated
-- per user row, and sitting beside a refresh token it turned a single leaked row into a
-- working Google credential. A refresh token alone mints nothing.
--
-- `config` is a JSON object, so the keys are removed with json_remove rather than by
-- rewriting the document; every other field (refresh_token, account_email, origins,
-- index_cap) is preserved exactly. Rows already written without the keys are untouched,
-- which makes this safe to run against a database that saw both code versions.
UPDATE workspace_mounts
SET config = json_remove(config, '$.client_id', '$.client_secret'),
    updated_at = updated_at
WHERE provider IN ('gmail', 'gdrive')
  AND json_valid(config)
  AND (
      json_extract(config, '$.client_id') IS NOT NULL
      OR json_extract(config, '$.client_secret') IS NOT NULL
  );
