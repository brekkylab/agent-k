export type UserId = string;
export type ProjectId = string;
export type SessionId = string;
export type ShareMode = 'private' | 'shared_readonly' | 'shared_chat';
export type SessionIntent = 'general' | 'analysis' | 'brainstorm' | 'writing' | 'recap';
export type RouteKey = 'projects' | 'project' | 'session' | 'files' | 'skills' | 'schedule' | 'members' | 'settings' | 'auth' | 'demo';

export interface User {
  id: UserId;
  name: string;
  roleLabel: string;
  avatar: string;
  color: string;
}

export interface Project {
  id: ProjectId;
  name: string;
  description: string;
  ownerId: UserId;
  memberIds: UserId[];
}

export type SessionOrigin = 'user' | 'automation';

export interface Session {
  id: SessionId;
  projectId: ProjectId;
  title: string;
  creatorId: UserId;
  shareMode: ShareMode;
  origin: SessionOrigin;
  intent: SessionIntent;
  updatedAt: string;
  lastMessageAt: string | null;
  lastMessageSnippet: string | null;
  unreadCount: number;
  references: FileAsset['id'][];
  artifactId?: string;
  isAutoAppend?: boolean;
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

export interface Message {
  id: string;
  sessionId: SessionId;
  sender: MessageSender;
  createdAt: string;
  body: string;
  toolCalls?: ToolCallInvocation[];
  citations?: FileAsset['id'][];
  attachments?: string[];
  artifacts?: string[];
  status?: 'sent' | 'streaming' | 'done';
}

export interface FileAsset {
  id: string;
  projectId: ProjectId;
  name: string;
  path: string;
  type: 'pdf' | 'sheet' | 'doc' | 'image' | 'folder';
  sizeLabel: string;
  updatedAt: string;
  summary: string;
  groundTruth: string[];
}

export interface Artifact {
  id: string;
  sessionId: SessionId;
  title: string;
  kind: 'team_decision_record';
  status: 'draft' | 'ready';
  generatedFromFileIds: FileAsset['id'][];
  sections: Array<{ label: string; body: string; evidence?: FileAsset['id'][] }>;
  nextActions: string[];
}

export interface SkillPreview {
  id: string;
  projectId: ProjectId;
  name: string;
  description: string;
  whenToUse: string;
  body: string;
  runnable: boolean;
  createdBy: UserId;
  createdAt: string;
  updatedAt: string;
  promptTemplate?: string;
  toolBindings?: string[];
  defaultIntent?: SessionIntent;
  sourceSessionId?: SessionId;
  sourceMessageRange?: { startTurn: number; endTurn: number };
}

export type ScheduleTrigger =
  | { kind: 'skill'; skillId: SkillPreview['id'] }
  | { kind: 'prompt'; prompt: string };

export type ScheduleResultTarget =
  | { kind: 'new_session_each_time' }
  | { kind: 'append_to_session'; sessionId: SessionId }
  | { kind: 'activity_feed_only' };

export interface SchedulePreview {
  id: string;
  projectId: ProjectId;
  cron: string;
  friendlyTime: string;
  timezone: string;
  active: boolean;
  createdBy: UserId;
  createdAt: string;
  trigger: ScheduleTrigger;
  resultTarget: ScheduleResultTarget;
  resultSessionShareMode?: ShareMode;
  notifyUserIds: UserId[];
  nextRunAt?: string;
}

export interface ActivityEntry {
  id: string;
  projectId: ProjectId;
  scheduleId?: SchedulePreview['id'];
  occurredAt: string;
  title: string;
  body: string;
}

export interface BootstrapPayload {
  users: User[];
  currentUserId: UserId;
  projects: Project[];
  sessions: Session[];
  messages: Message[];
  files: FileAsset[];
  artifacts: Artifact[];
  skills: SkillPreview[];
  schedules: SchedulePreview[];
  activityFeed: ActivityEntry[];
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
