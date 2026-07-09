import { ApiError, request, streamSse } from './client';
import type { MessageItem, RunEventPayload } from './types';

export async function listMessages(sessionId: string): Promise<MessageItem[]> {
  const result = await request<{ items: MessageItem[] }>(`/sessions/${sessionId}/messages`);
  return result.items;
}

export async function sendMessage(sessionId: string, text: string): Promise<void> {
  await request<void>(`/sessions/${sessionId}/messages`, {
    method: 'POST',
    body: { query: [{ type: 'text', text }] },
  });
}

export async function stopRun(sessionId: string): Promise<void> {
  try {
    await request<void>(`/sessions/${sessionId}/messages/stop`, { method: 'POST' });
  } catch (err) {
    // Swallow 404 (no active run), rethrow others.
    if (err instanceof ApiError && err.status === 404) return;
    throw err;
  }
}

export async function* streamSessionMessages(
  sessionId: string,
  lastSeq: number | undefined,
  signal: AbortSignal,
): AsyncGenerator<{ kind: 'message'; item: MessageItem } | { kind: 'run'; payload: RunEventPayload }> {
  const qs = lastSeq !== undefined ? `?last_seq=${lastSeq}` : '';
  const path = `/sessions/${sessionId}/messages/stream${qs}`;

  for await (const ev of streamSse(path, { method: 'GET', signal })) {
    // Parse inside try so a bad frame is skipped; yield outside so an
    // exception thrown into the generator by the consumer is not swallowed.
    let out:
      | { kind: 'message'; item: MessageItem }
      | { kind: 'run'; payload: RunEventPayload }
      | null = null;
    try {
      if (ev.event === 'message') {
        out = { kind: 'message', item: JSON.parse(ev.data) as MessageItem };
      } else if (ev.event === 'run') {
        out = { kind: 'run', payload: JSON.parse(ev.data) as RunEventPayload };
      }
      // Silently ignore unknown event names.
    } catch {
      // Silently ignore unparseable frames.
    }
    if (out) yield out;
  }
}
