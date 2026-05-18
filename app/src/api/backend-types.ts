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
  created_at: string;
  updated_at: string;
}

export interface BackendDirent {
  path: string;
  kind: 'file' | 'dir';
  bytes?: number | null;
  modified_at?: string | null;
}

export interface BackendUploadedFile { path: string; bytes: number; }
export interface BackendFailedFile { path: string; error: string; }
export interface BackendUploadResponse {
  project_id: string;
  succeeded: BackendUploadedFile[];
  failed: BackendFailedFile[];
}

export type AiloyPart =
  | { type: 'text'; text: string }
  | { type: 'value'; value: unknown }
  | { type: 'function'; function?: { name?: string; arguments?: unknown } }
  | { type?: string; [k: string]: unknown };

export interface AiloyMessage {
  id?: string | null;
  role: 'user' | 'assistant' | 'tool' | 'system' | string;
  contents?: AiloyPart[];
  tool_calls?: unknown[];
  thinking?: string | null;
}

export interface MessageOutput {
  depth: number;
  message: AiloyMessage;
  finish_reason?: { type?: string };
  usage?: { input_tokens?: number; output_tokens?: number };
}
