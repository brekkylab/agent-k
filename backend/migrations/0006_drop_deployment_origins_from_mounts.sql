-- Strip the deployment's service endpoints out of Gmail and Drive mount rows.
--
-- `origins` (where each Google service is reached) belongs to the installation, not to any
-- one mount: it was written from the backend's own config at mount-create, identical in
-- every row. Unlike the client pair that `0005` removed it is not a secret, but it carries
-- the OAuth token endpoint, so while it was read from the row, anyone able to write one
-- named the host that `client_id`, `client_secret` and `refresh_token` were POSTed to. The
-- backend now supplies it when it builds a mount, and nothing reads the stored copy.
--
-- All three keys go together so a row's content converges whether or not `0005` already
-- ran. Every other field (refresh_token, account_email, index_cap) is preserved exactly.
-- The `json_extract(...) IS NOT NULL` clause is load-bearing beyond selecting rows that
-- need work, because `json_valid` alone would admit a valid non-object config that
-- `json_remove` would rewrite.
--
-- This deliberately does not try to scrub the file. An UPDATE leaves the bytes it replaced
-- in the page, and neither `secure_delete` nor a rewrite of the same rows reaches what an
-- earlier UPDATE already freed -- measured, 20 rows on WAL: after `0005` and this one, 9 of
-- 20 client secrets were still recoverable with `strings`. Only a whole-file rewrite gets
-- to zero, and VACUUM is the wrong thing to put in front of a boot: it needs free space
-- equal to the database, it writes the whole database in the clear through a temp file in
-- TMPDIR, and if it fails the migration is never recorded, so every restart repeats it.
--
-- So the remedy is rotation, not deletion: on any database that ran the code before `0005` the
-- Google client secret must be treated as disclosed and the client rotated. An operator who
-- also wants the file scrubbed can `VACUUM` at a time of their choosing, where a failure
-- costs them a retry instead of the service.
UPDATE workspace_mounts
SET config = json_remove(config, '$.client_id', '$.client_secret', '$.origins')
WHERE provider IN ('gmail', 'gdrive')
  AND json_valid(config)
  AND (
      json_extract(config, '$.client_id') IS NOT NULL
      OR json_extract(config, '$.client_secret') IS NOT NULL
      OR json_extract(config, '$.origins') IS NOT NULL
  );
