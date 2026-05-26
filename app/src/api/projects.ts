import { request } from './client';
import type { BackendMember, BackendProject } from './backend-types';
import { resolveProjectId } from './projectId';
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

/** Accepts a project id OR a slug; backend route is id-only. */
export async function getProject(slugOrId: string): Promise<Project> {
  const projectId = await resolveProjectId(slugOrId);
  const raw = await request<BackendProject>(`/projects/${projectId}`);
  return toProject(raw);
}

/** Resolve a slug (active or retired) to the current project. */
export async function getProjectBySlug(slug: string): Promise<Project> {
  const raw = await request<BackendProject>(`/projects/by-slug/${slug}`);
  return toProject(raw);
}

export async function listMembers(slugOrId: string): Promise<User[]> {
  const projectId = await resolveProjectId(slugOrId);
  const res = await request<{ items: BackendMember[] }>(`/projects/${projectId}/members`);
  return res.items.map(toMemberUser);
}

export async function addMember(slugOrId: string, username: string): Promise<void> {
  const projectId = await resolveProjectId(slugOrId);
  await request(`/projects/${projectId}/members`, {
    method: 'POST',
    body: { username },
  });
}

export async function removeMember(slugOrId: string, userId: string): Promise<void> {
  const projectId = await resolveProjectId(slugOrId);
  await request(`/projects/${projectId}/members/${userId}`, { method: 'DELETE' });
}
