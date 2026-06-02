-- Model selection. Each session records which agent surface drives it and,
-- optionally, an explicit model pin. The effective model is resolved at
-- agent-build time, per session:
--   1. sessions.model                       (explicit pin; NULL = "recommended")
--   2. recommended chain for agent_type      → first model whose provider is
--      registered (has an API key) → falls back to moonshotai/kimi-k2.6
-- Forward-only, additive. Both columns nullable; existing rows resolve as
-- agent_type = coworker on the recommended chain.

-- Which agent surface drives the session: coworker | rag | deep-research | buddy.
-- Selects the recommendation chain (and, later, agent dispatch).
ALTER TABLE sessions ADD COLUMN agent_type TEXT;

-- Explicit model pin ("provider/model-id", e.g. "anthropic/claude-sonnet-4-6").
-- NULL = "recommended": resolve dynamically via the agent_type chain.
ALTER TABLE sessions ADD COLUMN model TEXT;

-- Per-project custom recommendation chains, as a JSON object keyed by
-- agent_type: {"coworker":["openai/gpt-5.4-mini", ...], ...}. A missing key (or
-- NULL column) uses the built-in default chain for that agent_type. Entries are
-- catalog model ids regardless of provider availability; resolution still walks
-- the chain and picks the first available (then the chain's last entry).
ALTER TABLE projects ADD COLUMN recommended_chains TEXT;

-- Per-automation agent surface + model selection, copied onto the session a
-- triggered run creates (where build_session_agent resolves them just like a
-- user session). Both nullable and additive:
--   agent_type NULL -> defaults to 'coworker' at agent-build time.
--   model      NULL -> "recommended" (resolved via the project's chain).
ALTER TABLE automations ADD COLUMN agent_type TEXT;
ALTER TABLE automations ADD COLUMN model TEXT;
