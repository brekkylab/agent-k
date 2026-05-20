import { request, streamSse } from './client';
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

export async function sendMessage(sessionId: string, content: string): Promise<MessageOutput[]> {
  return request<MessageOutput[]>(`/sessions/${sessionId}/messages`, {
    method: 'POST',
    body: { content },
  });
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

export async function* streamMessage(
  sessionId: string,
  content: string,
  signal?: AbortSignal,
): AsyncGenerator<StreamUpdate, void, void> {
  let accumulated = '';
  const toolCalls: StreamToolCall[] = [];
  const subagentTexts = new Map<string, string>();

  const currentSubagentUpdates = (): SubagentUpdate[] =>
    [...subagentTexts.entries()].map(([sourceAgent, text]) => ({ sourceAgent, text }));

  const snapshot = (): StreamUpdate => ({
    text: accumulated,
    toolCalls: [...toolCalls],
    status: 'streaming',
    subagentUpdates: currentSubagentUpdates(),
  });

  for await (const evt of streamSse(`/sessions/${sessionId}/messages/stream`, { content }, signal)) {
    if (evt.event === 'error') {
      yield { ...snapshot(), status: 'error', errorText: evt.data };
      return;
    }
    if (evt.event === 'done') {
      yield { ...snapshot(), status: 'done' };
      return;
    }
    if (evt.event !== 'message') continue;

    let output: MessageOutput | null = null;
    try { output = JSON.parse(evt.data) as MessageOutput; } catch { continue; }
    if (!output?.message) continue;

    const depth = output.depth ?? 0;
    const sourceAgent = output.source_agent ?? null;
    const msg = output.message as AiloyMessage;

    if (depth >= 1) {
      if (msg.role === 'assistant' && sourceAgent) {
        const text = aiMessageText(msg.contents as AiloyPart[] | undefined);
        subagentTexts.set(sourceAgent, (subagentTexts.get(sourceAgent) ?? '') + text);
        yield snapshot();
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
      yield snapshot();
    } else if (msg.role === 'tool') {
      if (!msg.id) {
        console.warn('[streamMessage] tool message without id; cannot attach', msg);
        continue;
      }
      const resultText = aiMessageText(msg.contents as AiloyPart[] | undefined) || '[done]';
      let tc = toolCalls.find((t) => t.id === msg.id);
      if (!tc) {
        console.warn(`[streamMessage] tool result id=${msg.id} arrived without matching tool_call; rendering as stub`);
        tc = { id: msg.id, name: '(pending)' };
        toolCalls.push(tc);
      }
      tc.result = resultText;
      yield snapshot();
    }
  }
}
