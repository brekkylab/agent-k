import { request } from './client';
import type { BackendSession } from './backend-types';
import { toSession } from './transformers';
import type { Session, ShareMode } from '@/domain/types';

export async function listSessions(projectId: string): Promise<Session[]> {
  const qs = projectId ? `?project_id=${encodeURIComponent(projectId)}` : '';
  const res = await request<{ items: BackendSession[] }>(`/sessions${qs}`);
  return res.items.map(toSession);
}

export async function createSession(projectId: string): Promise<Session> {
  const raw = await request<BackendSession>(`/sessions`, {
    method: 'POST',
    body: { project_id: projectId },
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
