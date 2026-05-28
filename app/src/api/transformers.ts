// Map raw backend payloads to the app-live domain types used in views.
// Default values for missing metadata (intent, references, etc.) are filled here.

export const SUBAGENT_PREFIX = 'subagent_';

import type { FileAsset, Message, MessageSender, PreferredLanguage, Project, Session, ToolCallInvocation, User } from '@/domain/types';
import type { AiloyPart, AiloyToolCall, BackendDirent, BackendMember, BackendProject, BackendSession, BackendUser, SessionMessageItem } from './backend-types';

function normalizeLanguage(value: string | undefined): PreferredLanguage {
  return value === 'ko' ? 'ko' : 'en';
}

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
    preferredLanguage: normalizeLanguage(backend.preferred_language),
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
    preferredLanguage: 'en',
  };
}

export const AI_USER: User = {
  id: 'ai',
  name: 'Cowork',
  roleLabel: 'Agent',
  avatar: 'CW',
  color: 'var(--cw-ink)',
  preferredLanguage: 'en',
};

export function toProject(backend: BackendProject, memberIds: string[] = []): Project {
  return {
    id: backend.id,
    slug: backend.slug,
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
    title: backend.title ?? '새 대화',
    creatorId: backend.creator_id,
    shareMode: backend.share_mode,
    origin: backend.origin,
    intent: 'general',
    updatedAt: compactDate(backend.updated_at),
    lastMessageAt: backend.last_message_at,
    lastMessageSnippet: backend.last_message_snippet,
    unreadCount: backend.unread_count,
    references: [],
  };
}

function extractText(contents: AiloyPart[] | undefined): string {
  if (!contents) return '';
  return contents
    .map((part) => {
      if (!part) return '';
      if (part.type === 'text') return (part as { text?: string }).text ?? '';
      if (part.type === 'value') {
        const val = (part as { value?: unknown }).value;
        return typeof val === 'string' ? val : safeStringify(val);
      }
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

export function toMessageItem(
  item: SessionMessageItem,
  sessionId: string,
  idx: number,
): Message {
  const a = item.message;
  const sender: MessageSender = item.sender.kind === 'user'
    ? { kind: 'user', userId: item.sender.user_id }
    : { kind: 'agent', name: item.sender.name };

  const toolCalls: ToolCallInvocation[] | undefined =
    a.role === 'assistant' && a.tool_calls?.length
      ? a.tool_calls.map((tc) => ({
          id: tc.id,
          name: tc.function?.name ?? 'tool',
          arguments: tc.function?.arguments,
        }))
      : undefined;

  const body = extractText(a.contents);

  return {
    id: a.id || `${sessionId}-h-${idx}`,
    sessionId,
    sender,
    createdAt: item.created_at,
    body,
    toolCalls,
    attachments: item.attachments ?? undefined,
    artifacts: item.artifacts?.length ? item.artifacts : undefined,
    status: 'done',
  };
}

export function collapseToolMessages(
  items: SessionMessageItem[],
  sessionId: string,
): Message[] {
  // tool_call_id → tool_call_name
  const toolCallNames = new Map<string, string>();
  for (const it of items) {
    if (it.message.role === 'assistant' && it.message.tool_calls) {
      for (const tc of it.message.tool_calls as AiloyToolCall[]) {
        toolCallNames.set(tc.id, tc.function?.name ?? 'tool');
      }
    }
  }

  // tool_call_id → result body (from role=tool messages shown as separate bubbles)
  const toolBodies = new Map<string, string>();
  for (const it of items) {
    if (it.message.role === 'tool' && it.message.id) {
      toolBodies.set(it.message.id, extractText(it.message.contents) || '[done]');
    }
  }

  const messages: Message[] = [];

  for (const [idx, it] of items.entries()) {
    if (it.message.role === 'tool') {
      const toolCallId = it.message.id;
      const toolName = toolCallId ? toolCallNames.get(toolCallId) : undefined;
      // Non-subagent tool results are inlined into the tool call <details> — skip the bubble.
      // Subagent results keep their own bubble (matches streaming behaviour).
      if (!toolName?.startsWith(SUBAGENT_PREFIX)) continue;

      const senderName = it.sender.kind === 'agent'
        ? it.sender.name
        : toolName ?? 'tool';
      messages.push({
        id: toolCallId || `${sessionId}-tool-${idx}`,
        sessionId,
        sender: { kind: 'agent' as const, name: senderName },
        createdAt: it.created_at,
        body: extractText(it.message.contents),
        status: 'done' as const,
      });
      continue;
    }

    const baseMsg = toMessageItem(it, sessionId, idx);
    if (baseMsg.toolCalls) {
      messages.push({
        ...baseMsg,
        toolCalls: baseMsg.toolCalls.map((tc) => ({
          ...tc,
          result: !tc.name.startsWith(SUBAGENT_PREFIX) && toolBodies.has(tc.id)
            ? toolBodies.get(tc.id)!
            : undefined,
        })),
      });
    } else {
      messages.push(baseMsg);
    }
  }

  return messages;
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
