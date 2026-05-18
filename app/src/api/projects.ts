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

export async function getProject(projectId: string): Promise<Project> {
  const raw = await request<BackendProject>(`/projects/${projectId}`);
  return toProject(raw);
}

export async function listMembers(projectId: string): Promise<User[]> {
  const res = await request<{ items: BackendMember[] }>(`/projects/${projectId}/members`);
  return res.items.map(toMemberUser);
}

export async function addMember(projectId: string, username: string): Promise<void> {
  await request(`/projects/${projectId}/members`, {
    method: 'POST',
    body: { username },
  });
}

export async function removeMember(projectId: string, userId: string): Promise<void> {
  await request(`/projects/${projectId}/members/${userId}`, { method: 'DELETE' });
}
