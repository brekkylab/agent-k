import { request } from './client';
import { getWorkspaceId } from '@/stores/workspace';
import type { SessionResponse, AgentType } from './types';

/** Max length for a session title, matching the backend clamp (auto + manual). */
export const SESSION_TITLE_MAX_LEN = 60;

export async function listSessions(): Promise<SessionResponse[]> {
  const result = await request<{ items: SessionResponse[] }>('/sessions');
  return result.items;
}

export async function createSession(input: {
  agentType: AgentType;
  title?: string;
  model?: string;
}): Promise<SessionResponse> {
  return request<SessionResponse>('/sessions', {
    method: 'POST',
    body: {
      workspace_id: getWorkspaceId(),
      agent_type: input.agentType,
      ...(input.title != null ? { title: input.title } : {}),
      ...(input.model != null ? { model: input.model } : {}),
    },
  });
}

export async function deleteSession(id: string): Promise<void> {
  await request<void>(`/sessions/${id}`, { method: 'DELETE' });
}

/** Rename a session. The server rejects an empty/whitespace-only title. */
export async function updateSessionTitle(id: string, title: string): Promise<SessionResponse> {
  return request<SessionResponse>(`/sessions/${id}`, {
    method: 'PATCH',
    body: { title },
  });
}
