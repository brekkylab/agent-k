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

export async function updateProject(
  projectRef: string,
  input: {
    name?: string;
    description?: string | null;
    recommendedChains?: Record<string, string[]>;
    pdfEngine?: string;
  },
): Promise<Project> {
  const body: Record<string, unknown> = {};
  if (input.name !== undefined) body.name = input.name;
  if (input.description !== undefined) body.description = input.description;
  // Send only when provided; `{}` resets all agents to defaults.
  if (input.recommendedChains !== undefined) body.recommended_chains = input.recommendedChains;
  if (input.pdfEngine !== undefined) body.pdf_engine = input.pdfEngine;
  const raw = await request<BackendProject>(`/projects/${projectRef}`, {
    method: 'PATCH',
    body,
  });
  return toProject(raw);
}

export interface KnowledgeStatus {
  indexing: boolean;
  documentCount: number;
}

/** Knowledge-corpus indexing status — whether a background resync is in flight. */
export async function getKnowledgeStatus(projectRef: string): Promise<KnowledgeStatus> {
  const raw = await request<{ indexing: boolean; document_count: number }>(
    `/projects/${projectRef}/knowledge/status`,
  );
  return { indexing: raw.indexing, documentCount: raw.document_count };
}

export async function deleteProject(projectRef: string): Promise<void> {
  await request(`/projects/${projectRef}`, { method: 'DELETE' });
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
