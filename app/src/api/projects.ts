import { request } from './client';
import type { BackendMember, BackendProject } from './backend-types';
import { toMemberUser, toProject } from './transformers';
import type { Project, User } from '@/domain/types';

export async function listProjects(): Promise<Project[]> {
  const res = await request<{ items: BackendProject[] }>('/projects');
  return res.items.map((p) => toProject(p));
}

export async function createProject(input: { name: string; description?: string }): Promise<Project> {
  const raw = await request<BackendProject>('/projects', {
    method: 'POST',
    body: { name: input.name, description: input.description ?? null },
  });
  return toProject(raw);
}

/** `projectRef` may be a UUID, an active slug, or a retired slug — backend resolves all three. */
export async function getProject(projectRef: string): Promise<Project> {
  const raw = await request<BackendProject>(`/projects/${projectRef}`);
  return toProject(raw);
}

export async function listMembers(projectRef: string): Promise<User[]> {
  const res = await request<{ items: BackendMember[] }>(`/projects/${projectRef}/members`);
  return res.items.map(toMemberUser);
}

export async function addMember(projectRef: string, username: string): Promise<void> {
  await request(`/projects/${projectRef}/members`, {
    method: 'POST',
    body: { username },
  });
}

export async function removeMember(projectRef: string, userId: string): Promise<void> {
  await request(`/projects/${projectRef}/members/${userId}`, { method: 'DELETE' });
}
