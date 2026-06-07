import { request } from './client';
import type { BackendSession } from './backend-types';
import { toSession } from './transformers';
import type { Session, SessionOrigin, ShareMode } from '@/domain/types';

export async function listSessions(projectRef: string, origin?: SessionOrigin): Promise<Session[]> {
  const params = new URLSearchParams({ project_ref: projectRef });
  if (origin) params.set('origin', origin);
  const res = await request<{ items: BackendSession[] }>(`/sessions?${params.toString()}`);
  return res.items.map(toSession);
}

export interface CreateSessionOptions {
  /** Agent surface (coworker | rag | deep-research | buddy). Omit for coworker. */
  agentType?: string;
  /** Explicit model pin ("provider/model-id"). Omit/null for recommended. */
  model?: string | null;
}

export async function createSession(
  projectRef: string,
  opts: CreateSessionOptions = {},
): Promise<Session> {
  const raw = await request<BackendSession>(`/sessions`, {
    method: 'POST',
    body: {
      project_ref: projectRef,
      ...(opts.agentType ? { agent_type: opts.agentType } : {}),
      ...(opts.model ? { model: opts.model } : {}),
    },
  });
  return toSession(raw);
}

export async function getSession(sessionId: string): Promise<Session> {
  const raw = await request<BackendSession>(`/sessions/${sessionId}`);
  return toSession(raw);
}

export async function updateSessionShareMode(sessionId: string, shareMode: ShareMode): Promise<Session> {
  const raw = await request<BackendSession>(`/sessions/${sessionId}`, {
    method: 'PATCH',
    body: { share_mode: shareMode },
  });
  return toSession(raw);
}

export async function deleteSession(sessionId: string): Promise<void> {
  await request(`/sessions/${sessionId}`, { method: 'DELETE' });
}

// User-facing label is "Duplicate" (the action clones the entire session
// — messages + sandbox); the backend endpoint is still `/fork` since that
// is the internal mechanism name.
export async function duplicateSession(sessionId: string): Promise<Session> {
  const raw = await request<BackendSession>(`/sessions/${sessionId}/fork`, {
    method: 'POST',
  });
  return toSession(raw);
}
