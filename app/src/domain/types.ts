export type UserId = string;
export type ProjectId = string;
export type SessionId = string;
export type ShareMode = 'private' | 'shared_readonly' | 'shared_chat';
export type PreferredLanguage = 'en' | 'ko';

export interface User {
  id: UserId;
  name: string;
  username?: string;
  roleLabel: string;
  avatar: string;
  color: string;
  preferredLanguage: PreferredLanguage;
}

export interface Project {
  id: ProjectId;
  slug: string;
  name: string;
  description: string;
  ownerId: UserId;
  memberIds: UserId[];
  /** Per-agent_type recommendation-chain overrides (agent_type → ordered model ids). */
  recommendedChains: Record<string, string[]>;
  /** Knowledge-corpus PDF engine: "kreuzberg" | "docling". */
  pdfEngine: string;
}

export type SessionOrigin = 'user' | 'automation';

export interface Session {
  id: SessionId;
  projectId: ProjectId;
  title: string;
  creatorId: UserId;
  shareMode: ShareMode;
  origin: SessionOrigin;
  updatedAt: string;
  lastMessageAt: string | null;
  lastMessageSnippet: string | null;
  unreadCount: number;
  /** True when an unread message mentions the current user. */
  unreadMention: boolean;
  references: FileAsset['id'][];
  isAutoAppend?: boolean;
  agentType: string | null;
  model: string | null;
  /** False when `model` is pinned but its provider key is gone (a fallback runs). */
  modelAvailable: boolean;
}

export type MessageSender =
  | { kind: 'user'; userId: UserId }
  | { kind: 'agent'; name: string };

export interface ToolCallInvocation {
  id: string;
  name: string;
  arguments?: unknown;
  result?: string;
}

export interface Citation {
  index: number;
  label: string;
  /** `missing` = a body marker `[^N]` with no `[^N]:` definition in Sources. */
  kind: 'corpus' | 'web' | 'missing' | string;
  verified: boolean;
  /** Whether a body marker cites this footnote (false = orphan definition). */
  referenced: boolean;
}

export interface Message {
  id: string;
  sessionId: SessionId;
  sender: MessageSender;
  createdAt: string;
  body: string;
  toolCalls?: ToolCallInvocation[];
  citations?: Citation[];
  attachments?: string[];
  artifacts?: string[];
  status?: 'sent' | 'streaming' | 'done';
  /** 'team' = user-to-user message, never delivered to the agent. Absent = normal chat. */
  messageKind?: 'team';
  mentions?: string[];
}

export interface FileAsset {
  id: string;
  projectId: ProjectId;
  name: string;
  path: string;
  type: 'pdf' | 'sheet' | 'doc' | 'image' | 'folder';
  sizeLabel: string;
  updatedAt: string;
}

// ── Automation domain types ────────────────────────────────────────────────
// camelCase mirrors of the backend Automation/Trigger/Run/RunEvent shapes.

export type AutomationId = string;
export type TriggerId = string;
export type RunId = string;

export type TriggerKind = 'cron' | 'webhook';
export type RunStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';
export type RunEventKind =
  | 'triggered' | 'queued' | 'started'
  | 'succeeded' | 'failed' | 'cancelled'
  | 'retry_scheduled' | 'retry_skipped'
  | 'lease_lost' | 'step_started' | 'step_finished';

/** Discriminated union mirroring backend TriggerSpec. */
export type TriggerSpec =
  | { kind: 'cron'; expr: string; tz: string | null }
  | { kind: 'webhook' };

export interface Automation {
  id: AutomationId;
  projectId: ProjectId;
  name: string;
  description: string | null;
  prompts: string[];
  enabled: boolean;
  agentType: string | null;
  model: string | null;
  createdBy: UserId;
  createdAt: string;
  updatedAt: string;
}

export interface Trigger {
  id: TriggerId;
  automationId: AutomationId;
  kind: TriggerKind;
  spec: TriggerSpec;
  enabled: boolean;
  nextFireAt: string | null;
  createdAt: string;
  updatedAt: string;
}

/** Returned only at trigger-creation time. `webhookToken` is the one-shot
 *  plaintext bearer token; subsequent reads from the API never expose it. */
export interface CreatedTrigger {
  trigger: Trigger;
  webhookToken: string | null;
}

export interface Run {
  id: RunId;
  automationId: AutomationId;
  triggerId: TriggerId | null;
  sessionId: SessionId;
  status: RunStatus;
  scheduledFor: string;
  leaseUntil: string | null;
  previousRunId: RunId | null;
  agentType: string | null;
  model: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface RunEvent {
  id: number;
  runId: RunId;
  ts: string;
  kind: RunEventKind;
  payload: unknown | null;
}
