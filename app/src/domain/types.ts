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

export interface Session {
  id: SessionId;
  projectId: ProjectId;
  title: string;
  creatorId: UserId;
  shareMode: ShareMode;
  intent: SessionIntent;
  updatedAt: string;
  references: FileAsset['id'][];
  artifactId?: string;
  isAutoAppend?: boolean;
}

export interface Message {
  id: string;
  sessionId: SessionId;
  senderId: UserId;
  createdAt: string;
  body: string;
  citations?: FileAsset['id'][];
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
