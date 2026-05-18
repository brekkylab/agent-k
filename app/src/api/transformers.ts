// Map raw backend payloads to the app-live domain types used in views.
// Default values for missing metadata (intent, references, etc.) are filled here.

import type { FileAsset, Message, Project, Session, User } from '@/domain/types';
import type { AiloyMessage, AiloyPart, BackendDirent, BackendMember, BackendProject, BackendSession, BackendUser } from './backend-types';

const USER_COLOR_TOKENS = [
  'var(--cw-cozy-clay)',
  'var(--cw-cozy-honey)',
  'var(--cw-cozy-mustard)',
  'var(--cw-cozy-sage)',
  'var(--cw-cozy-teal)',
  'var(--cw-cozy-plum)',
  'var(--cw-cozy-rose)',
];

function deterministicColor(seed: string): string {
  let hash = 0;
  for (let i = 0; i < seed.length; i += 1) hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  return USER_COLOR_TOKENS[hash % USER_COLOR_TOKENS.length]!;
}

function initials(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return 'U';
  const parts = trimmed.split(/\s+/);
  if (parts.length >= 2) return `${parts[0]![0]}${parts[1]![0]}`.toUpperCase();
  return trimmed.slice(0, 2).toUpperCase();
}

export function toUser(backend: BackendUser): User {
  const name = backend.display_name?.trim() || backend.username;
  return {
    id: backend.id,
    name,
    roleLabel: backend.role === 'admin' ? 'Admin' : 'Member',
    avatar: initials(name),
    color: deterministicColor(backend.id),
  };
}

export function toMemberUser(member: BackendMember): User {
  const name = member.display_name?.trim() || member.username;
  return {
    id: member.user_id,
    name,
    roleLabel: 'Member',
    avatar: initials(name),
    color: deterministicColor(member.user_id),
  };
}

export const AI_USER: User = {
  id: 'ai',
  name: 'Cowork',
  roleLabel: 'Agent',
  avatar: 'CW',
  color: 'var(--cw-ink)',
};

export function toProject(backend: BackendProject, memberIds: string[] = []): Project {
  return {
    id: backend.id,
    name: backend.name,
    description: backend.description || '',
    ownerId: backend.owner_id,
    memberIds: memberIds.length > 0 ? memberIds : [backend.owner_id],
  };
}

export function toSession(backend: BackendSession): Session {
  return {
    id: backend.id,
    projectId: backend.project_id,
    title: `Session ${backend.id.slice(0, 8)}`,
    creatorId: backend.creator_id,
    shareMode: backend.share_mode,
    intent: 'general',
    updatedAt: compactDate(backend.updated_at),
    references: [],
  };
}

function extractText(contents: AiloyPart[] | undefined): string {
  if (!contents) return '';
  return contents
    .map((part) => {
      if (!part) return '';
      if (part.type === 'text') return (part as { text?: string }).text ?? '';
      if (part.type === 'value') return safeStringify((part as { value?: unknown }).value);
      if (part.type === 'function') {
        const fn = (part as { function?: { name?: string } }).function;
        return fn?.name ? `[tool: ${fn.name}]` : '[tool call]';
      }
      return '';
    })
    .filter(Boolean)
    .join('\n');
}

function safeStringify(value: unknown): string {
  try { return JSON.stringify(value, null, 2); } catch { return String(value); }
}

export function aiMessageText(contents: AiloyPart[] | undefined): string {
  return extractText(contents);
}

export function toMessage(ailoy: AiloyMessage, sessionId: string, index: number, fallbackSender: string): Message {
  const role = ailoy.role;
  const isAssistant = role === 'assistant' || role === 'tool';
  return {
    id: ailoy.id || `${sessionId}-h-${index}`,
    sessionId,
    senderId: isAssistant ? 'ai' : fallbackSender,
    createdAt: '이전 대화',
    body: extractText(ailoy.contents) || (role === 'tool' ? '[tool result]' : ''),
    status: 'done',
  };
}

export function toFileAsset(entry: BackendDirent, projectId: string, projectName: string): FileAsset {
  const segments = entry.path.split('/').filter(Boolean);
  const name = segments.at(-1) ?? entry.path;
  return {
    id: `${projectId}:${entry.path}`,
    projectId,
    name: entry.kind === 'dir' ? `${name}/` : name,
    path: `${projectName}/${entry.path}`,
    type: entry.kind === 'dir' ? 'folder' : inferFileType(entry.path),
    sizeLabel: entry.kind === 'dir' ? 'folder' : formatBytes(entry.bytes ?? 0),
    updatedAt: entry.modified_at ? compactDate(entry.modified_at) : '—',
    summary: '',
    groundTruth: entry.kind === 'dir'
      ? ['Persisted directory']
      : ['Persisted file in backend storage'],
  };
}

function inferFileType(path: string): FileAsset['type'] {
  const lower = path.toLowerCase();
  if (lower.endsWith('.pdf')) return 'pdf';
  if (lower.endsWith('.xlsx') || lower.endsWith('.csv') || lower.endsWith('.tsv')) return 'sheet';
  if (/\.(png|jpe?g|webp|gif|svg)$/i.test(lower)) return 'image';
  return 'doc';
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function compactDate(value: string | null | undefined): string {
  if (!value) return '—';
  return value.slice(0, 10);
}
