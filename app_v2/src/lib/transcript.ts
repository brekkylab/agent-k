// Slim port of extractText/aiMessageText and tool-call collapse from
// /Users/jeffrey/workspace/agent-k/app/src/api/transformers.ts.
// No team-message, mention, or attachment handling.

import type { AiloyPart, AiloyToolCall, MessageItem } from '@/api/types';

export interface ToolCallEntry {
  id: string;
  name: string;
  arguments?: unknown;
  result?: string;
}

export interface TranscriptEntry {
  kind: 'user' | 'assistant';
  text: string;
  toolCalls: ToolCallEntry[];
  /** RFC3339 timestamp of the underlying message; '' for optimistic entries. */
  createdAt: string;
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

export function messageItemText(item: MessageItem): string {
  return extractText(item.message.contents);
}

export function buildTranscript(items: MessageItem[]): TranscriptEntry[] {
  // Map tool_call_id → tool name from assistant messages.
  const toolCallNames = new Map<string, string>();
  for (const it of items) {
    if (it.message.role === 'assistant' && it.message.tool_calls) {
      for (const tc of it.message.tool_calls as AiloyToolCall[]) {
        toolCallNames.set(tc.id, tc.function?.name ?? 'tool');
      }
    }
  }

  // Map tool_call_id → result from role=tool messages.
  const toolResults = new Map<string, string>();
  for (const it of items) {
    if (it.message.role === 'tool' && it.message.id) {
      toolResults.set(it.message.id, extractText(it.message.contents) || '[done]');
    }
  }

  const entries: TranscriptEntry[] = [];

  for (const it of items) {
    const role = it.message.role;

    // Skip tool result messages — they are inlined into the assistant tool-call entry.
    if (role === 'tool') continue;

    const text = extractText(it.message.contents);
    const kind: 'user' | 'assistant' = role === 'user' ? 'user' : 'assistant';

    const toolCalls: ToolCallEntry[] = (it.message.tool_calls ?? []).map((tc) => ({
      id: tc.id,
      name: tc.function?.name ?? 'tool',
      arguments: tc.function?.arguments,
      result: toolResults.get(tc.id),
    }));

    entries.push({ kind, text, toolCalls, createdAt: it.created_at });
  }

  return entries;
}
