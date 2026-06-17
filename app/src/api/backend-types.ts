// Raw response shapes coming from backend-v2 (axum + sqlx + aide).
// These never leak into view code — transformers.ts maps them onto domain types.

import type { ShareMode } from '@/domain/types';

export interface BackendUser {
  id: string;
  username: string;
  display_name?: string | null;
  role?: 'admin' | 'user' | string;
  is_active?: boolean;
  preferred_language?: string;
  created_at?: string;
  updated_at?: string;
}

export interface LoginResponse {
  access_token: string;
  token_type: string;
  expires_in: number;
  user: BackendUser;
}

export interface BackendProject {
  id: string;
  slug: string;
  name: string;
  description?: string | null;
  owner_id: string;
  /** Per-agent_type recommendation-chain overrides; only customized agents present. */
  recommended_chains?: Record<string, string[]>;
  /** Knowledge-corpus PDF engine: "kreuzberg" | "docling". */
  pdf_engine?: string;
  created_at: string;
  updated_at: string;
}

export interface BackendMember {
  user_id: string;
  username: string;
  display_name?: string | null;
  added_at: string;
}

export type BackendSessionOrigin = 'user' | 'automation';

export interface BackendSession {
  id: string;
  project_id: string;
  creator_id: string;
  share_mode: ShareMode;
  origin: BackendSessionOrigin;
  title: string | null;
  last_message_at: string | null;
  last_message_snippet: string | null;
  agent_type: string | null;
  model: string | null;
  model_available: boolean;
  unread_count: number;
  created_at: string;
  updated_at: string;
}

export interface BackendDirent {
  path: string;
  kind: 'file' | 'dir';
  bytes?: number | null;
  modified_at?: string | null;
}

export interface BackendFailedFile { path: string; error: string; }

/// Unified result shape for upload / move / copy batch operations.
export interface BackendDirentBatchResult {
  succeeded: BackendDirent[];
  failed: BackendFailedFile[];
}

/// Tagged union for PATCH /dirents batch operations.
export type BackendDirentBatchOp =
  | { op: 'move'; sources: string[]; destination: string; new_name: string | null }
  | { op: 'copy'; sources: string[]; destination: string };

export type AiloyPart =
  | { type: 'text'; text: string }
  | { type: 'value'; value: unknown }
  | { type: 'function'; function?: { name?: string; arguments?: unknown } }
  | { type?: string; [k: string]: unknown };

export interface AiloyToolCall {
  id: string;
  type?: 'function' | string;
  function: { name: string; arguments?: unknown };
}

export interface AiloyMessage {
  id?: string | null;
  role: 'user' | 'assistant' | 'tool' | 'system' | string;
  contents?: AiloyPart[];
  tool_calls?: AiloyToolCall[];
  thinking?: string | null;
}

export type BackendMessageSender =
  | { kind: 'user'; user_id: string }
  | { kind: 'agent'; name: string };

export interface BackendCitation {
  index: number;
  label: string;
  /** `missing` = a body marker `[^N]` with no `[^N]:` definition in Sources. */
  kind: 'corpus' | 'web' | 'missing' | string;
  verified: boolean;
  /** Whether a body marker cites this footnote (false = orphan definition). */
  referenced: boolean;
}

export interface SessionMessageItem {
  /** Session-global insertion order — stable identity across paginated windows. */
  seq: number;
  message: AiloyMessage;
  sender: BackendMessageSender;
  created_at: string;
  attachments?: string[];
  artifacts?: string[];
  citations?: BackendCitation[];
}

export interface SessionMessageList {
  items: SessionMessageItem[];
}

export interface MessageOutput {
  depth?: number | null;
  source_agent?: string | null;
  message: AiloyMessage;
  finish_reason?: { type?: string };
  usage?: { input_tokens?: number; output_tokens?: number };
}

// ── Automation ─────────────────────────────────────────────────────────────
// Mirrors backend/src/model/automation.rs. snake_case is preserved here;
// transformers.ts maps these onto camelCase domain types.

export interface BackendAutomation {
  id: string;
  project_id: string;
  name: string;
  description: string | null;
  prompts: string[];
  enabled: boolean;
  agent_type: string | null;
  model: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface BackendAutomationList {
  items: BackendAutomation[];
}

export interface CreateAutomationRequest {
  project_id: string;
  name: string;
  description?: string | null;
  prompts: string[];
  agent_type?: string | null;
  model?: string | null;
}

export interface UpdateAutomationRequest {
  name?: string;
  description?: string | null;
  prompts?: string[];
  enabled?: boolean;
  agent_type?: string | null;
  model?: string | null;
}

// ── Trigger ────────────────────────────────────────────────────────────────

export type BackendTriggerKind = 'cron' | 'webhook';

/** API-shape: internally tagged by `kind`. */
export type BackendTriggerSpec =
  | { kind: 'cron'; expr: string; tz?: string | null }
  | { kind: 'webhook' };

export interface BackendTrigger {
  id: string;
  automation_id: string;
  kind: BackendTriggerKind;
  spec: BackendTriggerSpec;
  enabled: boolean;
  next_fire_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface BackendTriggerList {
  items: BackendTrigger[];
}

/** POST body is the spec directly (no wrapper). */
export type CreateTriggerRequest = BackendTriggerSpec;

export interface UpdateTriggerRequest {
  spec?: BackendTriggerSpec;
  enabled?: boolean;
}

/**
 * Creation response. For webhook triggers, `webhook_token` is the only
 * chance to read the plaintext bearer token — subsequent GETs never include
 * it (only the masked preview via the trigger's other fields, if any).
 */
export interface CreatedTriggerResponse {
  trigger: BackendTrigger;
  webhook_token?: string;
}

// ── Run ────────────────────────────────────────────────────────────────────

export type BackendRunStatus =
  | 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';

export interface BackendRun {
  id: string;
  automation_id: string;
  trigger_id: string | null;
  session_id: string;
  status: BackendRunStatus;
  scheduled_for: string;
  lease_until: string | null;
  previous_run_id: string | null;
  agent_type: string | null;
  model: string | null;
  created_at: string;
  updated_at: string;
}

export interface BackendRunList {
  items: BackendRun[];
}

/** POST /automations/:id/runs body — currently no payload. */
export interface CreateRunRequest {}

// ── Run event ──────────────────────────────────────────────────────────────

export type BackendEventKind =
  | 'triggered' | 'queued' | 'started'
  | 'succeeded' | 'failed' | 'cancelled'
  | 'retry_scheduled' | 'retry_skipped'
  | 'lease_lost' | 'step_started' | 'step_finished';

export interface BackendRunEvent {
  id: number;
  run_id: string;
  ts: string;
  kind: BackendEventKind;
  /** Shape varies per kind; treat as unknown JSON. */
  payload: unknown | null;
}

export interface BackendRunEventList {
  items: BackendRunEvent[];
}
