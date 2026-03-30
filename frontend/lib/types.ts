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

export interface SessionSource {
  name: string;
  size: number;
}

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
  createdAt: Date;
}

export interface ChatSession {
  id: string;
  title: string;
  messages: ChatMessage[];
  knowledgeIds: string[];
  sessionSources: SessionSource[];
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

export interface ApiSessionMessage {
  role: "system" | "user" | "assistant" | "tool";
  content: string;
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
