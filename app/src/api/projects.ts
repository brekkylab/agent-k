import { request } from './client';
import type { BackendMember, BackendProject } from './backend-types';
import { toProject, toProjectMember } from './transformers';
import type { Project, ProjectMember } from '@/domain/types';

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
  /** Null while the store is locked by an in-flight resync (count unknown then). */
  documentCount: number | null;
  /** Last resync error, if the most recent resync failed; null otherwise. */
  error: string | null;
}

/** Knowledge-corpus indexing status — whether a background resync is in flight. */
export async function getKnowledgeStatus(projectRef: string): Promise<KnowledgeStatus> {
  const raw = await request<{ indexing: boolean; document_count: number | null; error: string | null }>(
    `/projects/${projectRef}/knowledge/status`,
  );
  return { indexing: raw.indexing, documentCount: raw.document_count ?? null, error: raw.error ?? null };
}

export interface KnowledgeFileStatus {
  /** Scope-relative path, e.g. `knowledge/report.pdf`. */
  path: string;
  indexed: boolean;
  /** The latest resync tried and failed to index this file. */
  failed: boolean;
}

export interface KnowledgeFiles {
  files: KnowledgeFileStatus[];
  indexing: boolean;
}

/** Per-file corpus status for the knowledge folder. */
export async function getKnowledgeFiles(projectRef: string): Promise<KnowledgeFiles> {
  return request<KnowledgeFiles>(`/projects/${projectRef}/knowledge/files`);
}

export async function deleteProject(projectRef: string): Promise<void> {
  await request(`/projects/${projectRef}`, { method: 'DELETE' });
}

export async function listMembers(projectRef: string): Promise<ProjectMember[]> {
  const res = await request<{ items: BackendMember[] }>(`/projects/${projectRef}/members`);
  return res.items.map(toProjectMember);
}

/**
 * Set the project's agent-capability ceiling (owner only).
 * `null` clears the ceiling (no limit — all capabilities allowed).
 */
export async function setProjectAgentCeiling(
  projectRef: string,
  capabilities: string[] | null,
): Promise<Project> {
  const raw = await request<BackendProject>(`/projects/${projectRef}/agent-ceiling`, {
    method: 'PATCH',
    body: { capabilities },
  });
  return toProject(raw);
}

/**
 * Set the current member's own per-project agent grant (self only — 403 otherwise).
 * `null` resets to inherit the project ceiling.
 */
export async function setMemberAgentCapabilities(
  projectRef: string,
  userId: string,
  capabilities: string[] | null,
): Promise<void> {
  await request(`/projects/${projectRef}/members/${userId}/agent-capabilities`, {
    method: 'PATCH',
    body: { capabilities },
  });
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
