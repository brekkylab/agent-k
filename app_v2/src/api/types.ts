// Backend_v2 DTO mirrors — shared contract for all API modules.

export interface UserResponse {
  id: string;
  username: string;
  role: string;
  display_name: string | null;
  is_active: boolean;
  preferred_language: string;
  created_at: string;
  updated_at: string;
}

export interface LoginResponse {
  access_token: string;
  token_type: string;
  expires_in: number;
  user: UserResponse;
}

export interface WorkspaceResponse {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
}

export interface AiloyPart {
  type?: string;
  text?: string;
  [k: string]: unknown;
}

export interface AiloyToolCall {
  id: string;
  function: { name: string; arguments?: unknown };
  [k: string]: unknown;
}

export interface AiloyMessage {
  id?: string | null;
  role: string;
  contents?: AiloyPart[];
  tool_calls?: AiloyToolCall[];
  thinking?: string | null;
  [k: string]: unknown;
}

export interface SessionResponse {
  id: string;
  workspace_id: string;
  agent_id: string | null;
  title: string | null;
  spec: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export type AgentType = 'coworker' | 'deep_research';

export interface MessageItem {
  seq: number;
  message: AiloyMessage;
  /** RFC3339 timestamp of when the message was persisted. */
  created_at: string;
}

export type RunEventPayload =
  | { run: 'started' | 'finished' | 'idle' }
  | { run: 'error'; message: string };

/// `title` SSE frame: an auto-generated session title, published once when an
/// untitled session's title is first persisted (concurrent with its run).
export interface TitleEventPayload {
  title: string;
}
