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

export async function getProject(slug: string): Promise<Project> {
  const raw = await request<BackendProject>(`/projects/${slug}`);
  return toProject(raw);
}

export async function listMembers(slug: string): Promise<User[]> {
  const res = await request<{ items: BackendMember[] }>(`/projects/${slug}/members`);
  return res.items.map(toMemberUser);
}

export async function addMember(slug: string, username: string): Promise<void> {
  await request(`/projects/${slug}/members`, {
    method: 'POST',
    body: { username },
  });
}

export async function removeMember(slug: string, userId: string): Promise<void> {
  await request(`/projects/${slug}/members/${userId}`, { method: 'DELETE' });
}
