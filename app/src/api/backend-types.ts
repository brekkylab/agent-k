// Raw response shapes coming from backend-v2 (axum + sqlx + aide).
// These never leak into view code — transformers.ts maps them onto domain types.

import type { ShareMode } from '@/domain/types';

export interface BackendUser {
  id: string;
  username: string;
  display_name?: string | null;
  role?: 'admin' | 'user' | string;
  is_active?: boolean;
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
  name: string;
  description?: string | null;
  owner_id: string;
  created_at: string;
  updated_at: string;
}

export interface BackendMember {
  user_id: string;
  username: string;
  display_name?: string | null;
  added_at: string;
}

export interface BackendSession {
  id: string;
  project_id: string;
  creator_id: string;
  share_mode: ShareMode;
  title: string | null;
  last_message_at: string | null;
  last_message_snippet: string | null;
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
  project_id: string;
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

export interface SessionMessageItem {
  message: AiloyMessage;
  sender: BackendMessageSender;
  created_at: string;
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
