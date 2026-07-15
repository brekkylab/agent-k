-- Optional user-chosen display name for a mount, distinct from `prefix` (which
-- stays the path-safe routing segment). Nullable: existing rows backfill to
-- NULL, and the UI falls back to the prefix when it's absent. Additive only —
-- routing/config are unchanged, so a mount with no label behaves as before.
ALTER TABLE workspace_mounts ADD COLUMN label TEXT;
