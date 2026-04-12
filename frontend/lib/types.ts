// --- Backend API Source/Speedwagon Types ---

export interface ApiSource {
  id: string;
  name: string;
  source_type: string;
  size: number;
  created_at: string;
  updated_at: string;
}

export type SpeedwagonIndexStatus = "not_indexed" | "indexing" | "indexed" | "error";

export interface ApiSpeedwagon {
  id: string;
  name: string;
  description: string;
  instruction: string | null;
  lm: string | null;
  provider_profile_id: string | null;
  source_ids: string[];
  index_dir: string | null;
  corpus_dir: string | null;
  index_status: SpeedwagonIndexStatus;
  index_error: string | null;
  index_started_at: string | null;
  indexed_at: string | null;
  created_at: string;
  updated_at: string;
}

// --- Backend API Response Types ---

export interface ApiProviderProfile {
  id: string;
  name: string;
  provider: {
    lm: {
      type: "api";
      schema: string;
      url: string;
      api_key?: string;
    };
    tools: unknown[];
  };
  is_default: boolean;
  created_at: string;
  updated_at: string;
}

export interface ApiAgent {
  id: string;
  spec: {
    lm: string;
    instruction?: string;
    tools?: unknown[];
  };
  created_at: string;
  updated_at: string;
}

export interface ApiSessionToolCall {
  id: string;
  message_id: string;
  tool_name: string;
  tool_args: Record<string, unknown> | null;
  tool_result: unknown | null;
  duration_ms: number | null;
  created_at: string;
}

export interface ApiSessionMessage {
  id: string;
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  tool_calls?: ApiSessionToolCall[];
  created_at: string;
}

export interface ApiSession {
  id: string;
  agent_id: string;
  provider_profile_id: string;
  title: string | null;
  messages?: ApiSessionMessage[];
  speedwagon_ids: string[];
  source_ids: string[];
  created_at: string;
  updated_at: string;
}

// --- SSE Streaming Types ---

export type SseEventType = "thinking" | "tool_call" | "tool_result" | "message" | "done" | "error";

export interface SseEvent {
  type: string;
  level?: string;
  tool?: string;
  args?: Record<string, unknown>;
  result?: unknown;
  error?: string;
  content?: string;
  message?: string;
  assistant_message?: ApiSessionMessage;
}

// --- Request Types ---

export interface CreateProviderProfileRequest {
  name: string;
  provider: ApiProviderProfile["provider"];
  is_default?: boolean;
}

export interface UpdateProviderProfileRequest {
  name: string;
  provider: ApiProviderProfile["provider"];
  is_default?: boolean;
}

export interface CreateAgentRequest {
  spec: {
    lm: string;
    instruction?: string;
    tools?: unknown[];
  };
}

export interface UpdateAgentRequest {
  spec: {
    lm: string;
    instruction?: string;
    tools?: unknown[];
  };
}

export interface CreateSessionRequest {
  agent_id: string;
  provider_profile_id?: string;
  title?: string;
  speedwagon_ids?: string[];
  source_ids?: string[];
}

export interface UpdateSessionRequest {
  title?: string;
  agent_id?: string;
  provider_profile_id?: string;
  speedwagon_ids?: string[];
  source_ids?: string[];
}

export interface CreateSpeedwagonRequest {
  name: string;
  description: string;
  instruction?: string | null;
  lm?: string | null;
  provider_profile_id?: string | null;
  source_ids?: string[];
}

export interface UpdateSpeedwagonRequest {
  name: string;
  description: string;
  instruction?: string | null;
  lm?: string | null;
  provider_profile_id?: string | null;
  source_ids: string[];
}
