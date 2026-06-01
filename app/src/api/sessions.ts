import { request } from './client';
import type { BackendSession } from './backend-types';
import { toSession } from './transformers';
import type { Session, ShareMode } from '@/domain/types';

export async function listSessions(projectRef: string): Promise<Session[]> {
  const res = await request<{ items: BackendSession[] }>(`/sessions?project_ref=${encodeURIComponent(projectRef)}`);
  return res.items.map(toSession);
}

export async function createSession(projectRef: string): Promise<Session> {
  const raw = await request<BackendSession>(`/sessions`, {
    method: 'POST',
    body: { project_ref: projectRef },
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
