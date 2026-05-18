import { request, streamSse } from './client';
import type { AiloyMessage, AiloyPart, MessageOutput } from './backend-types';
import { aiMessageText, toMessage } from './transformers';
import type { Message } from '@/domain/types';

export async function listMessages(sessionId: string, fallbackSender: string): Promise<Message[]> {
  const raw = await request<AiloyMessage[]>(`/sessions/${sessionId}/messages`);
  return raw.map((m, idx) => toMessage(m, sessionId, idx, fallbackSender));
}

export async function sendMessage(sessionId: string, content: string): Promise<MessageOutput[]> {
  return request<MessageOutput[]>(`/sessions/${sessionId}/messages`, {
    method: 'POST',
    body: { content },
  });
}

export interface StreamUpdate {
  text: string;
  toolCalls: string[];
  status: 'streaming' | 'done' | 'error';
  errorText?: string;
}

export async function* streamMessage(
  sessionId: string,
  content: string,
  signal?: AbortSignal,
): AsyncGenerator<StreamUpdate, void, void> {
  let accumulated = '';
  const toolCalls: string[] = [];

  for await (const evt of streamSse(`/sessions/${sessionId}/messages/stream`, { content }, signal)) {
    if (evt.event === 'error') {
      yield { text: accumulated, toolCalls, status: 'error', errorText: evt.data };
      return;
    }
    if (evt.event === 'done') {
      yield { text: accumulated, toolCalls, status: 'done' };
      return;
    }
    if (evt.event !== 'message') continue;

    let output: { message?: AiloyMessage } | null = null;
    try { output = JSON.parse(evt.data) as { message?: AiloyMessage }; } catch { continue; }
    if (!output?.message) continue;

    if (output.message.role === 'assistant') {
      accumulated = aiMessageText(output.message.contents as AiloyPart[] | undefined);
      const calls = (output.message.tool_calls ?? []) as Array<{ function?: { name?: string } }>;
      for (const call of calls) {
        const name = call?.function?.name;
        if (name && !toolCalls.includes(name)) toolCalls.push(name);
      }
      yield { text: accumulated, toolCalls, status: 'streaming' };
    }
  }
}
