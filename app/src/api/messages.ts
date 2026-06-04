import { request } from './client';
import type { AiloyMessage, AiloyPart, AiloyToolCall, MessageOutput, SessionMessageList } from './backend-types';
import { aiMessageText, collapseToolMessages } from './transformers';
import type { Message } from '@/domain/types';

export interface SubagentUpdate {
  sourceAgent: string;
  text: string;
}

export async function listMessages(sessionId: string): Promise<Message[]> {
  const raw = await request<SessionMessageList>(`/sessions/${sessionId}/messages`);
  return collapseToolMessages(raw.items, sessionId);
}

export interface RunAck {
  status: string;
  run_id: string;
}

export async function sendMessage(
  sessionId: string,
  content: string,
  attachments?: string[],
): Promise<RunAck> {
  return request<RunAck>(`/sessions/${sessionId}/messages`, {
    method: 'POST',
    body: { content, attachments: attachments && attachments.length > 0 ? attachments : undefined },
  });
  // 423/403 are thrown as ApiError by request() — caller should catch
}

export async function stopRun(sessionId: string, runId: string): Promise<void> {
  await request<unknown>(`/sessions/${sessionId}/runs/${runId}/stop`, { method: 'POST' });
}

export interface StreamToolCall {
  id: string;
  name: string;
  arguments?: unknown;
  result?: string;
}

export interface StreamUpdate {
  text: string;
  toolCalls: StreamToolCall[];
  status: 'streaming' | 'done' | 'error';
  errorText?: string;
  subagentUpdates: SubagentUpdate[];
}

/**
 * Derives a StreamUpdate snapshot from an ordered list of MessageOutput items.
 * Items must be sorted by seq (ascending). Pure — no side effects.
 *
 * NOTE: For assistant messages, `accumulated` is replaced (not appended) on each
 * item. This is intentional: the backend sends complete text snapshots per message,
 * not incremental deltas. Passing items where two sequential assistant messages
 * represent separate text segments would silently drop the first.
 */
export function deriveStreamState(
  outputs: MessageOutput[],
  status: 'streaming' | 'done' | 'error' = 'streaming',
): StreamUpdate {
  let accumulated = '';
  const toolCalls: StreamToolCall[] = [];
  const subagentTexts = new Map<string, string>();

  for (const output of outputs) {
    if (!output?.message) continue;

    const depth = output.depth ?? 0;
    const sourceAgent = output.source_agent ?? null;
    const msg = output.message as AiloyMessage;

    if (depth >= 1) {
      if (msg.role === 'assistant' && sourceAgent) {
        const text = aiMessageText(msg.contents as AiloyPart[] | undefined);
        subagentTexts.set(sourceAgent, (subagentTexts.get(sourceAgent) ?? '') + text);
      }
      continue;
    }

    if (msg.role === 'assistant') {
      accumulated = aiMessageText(msg.contents as AiloyPart[] | undefined);
      for (const call of (msg.tool_calls ?? []) as AiloyToolCall[]) {
        if (!call.id || !call.function?.name) continue;
        const existing = toolCalls.find((tc) => tc.id === call.id);
        if (existing) {
          if (existing.name === '(pending)') existing.name = call.function.name;
          if (existing.arguments === undefined) existing.arguments = call.function.arguments;
        } else {
          toolCalls.push({ id: call.id, name: call.function.name, arguments: call.function.arguments });
        }
      }
    } else if (msg.role === 'tool') {
      if (!msg.id) continue;
      const resultText = aiMessageText(msg.contents as AiloyPart[] | undefined) || '[done]';
      let tc = toolCalls.find((t) => t.id === msg.id);
      if (!tc) {
        console.warn(`[deriveStreamState] tool result id=${msg.id} arrived without matching tool_call; rendering as stub`);
        tc = { id: msg.id, name: '(pending)' };
        toolCalls.push(tc);
      }
      tc.result = resultText;
    }
  }

  const subagentUpdates: SubagentUpdate[] = [...subagentTexts.entries()].map(([sourceAgent, text]) => ({ sourceAgent, text }));

  return {
    text: accumulated,
    toolCalls: [...toolCalls],
    status,
    subagentUpdates,
  };
}
