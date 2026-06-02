import { ApiError, BASE_URL, getToken, notifyUnauthorized, request } from './client';
import type { BackendDirent, BackendDirentBatchOp, BackendDirentBatchResult } from './backend-types';
import { toFileAsset } from './transformers';
import type { FileAsset } from '@/domain/types';

export type DirentBatchResult = BackendDirentBatchResult;

export type DirentScope =
  | { kind: 'shared'; projectId: string }
  | { kind: 'inputs'; projectId: string; sessionId: string }
  | { kind: 'artifacts'; projectId: string; sessionId: string };

export function scopeRoot(s: DirentScope): string {
  switch (s.kind) {
    case 'shared':    return `projects/${s.projectId}/shared`;
    case 'inputs':    return `projects/${s.projectId}/sessions/${s.sessionId}/inputs`;
    case 'artifacts': return `projects/${s.projectId}/sessions/${s.sessionId}/artifacts`;
  }
}

/** Strip the scope-root prefix from a global path, returning scope-relative path. */
export function stripScopePrefix(s: DirentScope, path: string): string {
  const root = scopeRoot(s);
  if (path === root) return '';
  if (path.startsWith(root + '/')) return path.slice(root.length + 1);
  return path;
}

function encodePath(path: string): string {
  return path.split('/').map(encodeURIComponent).join('/');
}

export async function listDirents(
  scope: DirentScope,
  projectName: string,
  recursive = true,
): Promise<FileAsset[]> {
  const res = await request<{ entries: BackendDirent[] }>(
    `/dirents?path=${scopeRoot(scope)}&recursive=${recursive}`,
  );
  const projectId = scope.projectId;
  return res.entries.map((e) => {
    const rel = stripScopePrefix(scope, e.path);
    return toFileAsset({ ...e, path: rel }, projectId, projectName);
  });
}

export async function listDirentsRaw(
  scope: DirentScope,
  recursive = true,
): Promise<BackendDirent[]> {
  const res = await request<{ entries: BackendDirent[] }>(
    `/dirents?path=${scopeRoot(scope)}&recursive=${recursive}`,
  );
  return res.entries;
}

export async function uploadFile(
  scope: DirentScope,
  file: File,
  targetPath: string,
): Promise<DirentBatchResult> {
  return uploadFiles(scope, [{ file, targetPath }]);
}

export async function uploadFiles(
  scope: DirentScope,
  items: Array<{ file: File; targetPath: string }>,
): Promise<DirentBatchResult> {
  const form = new FormData();
  for (const { file, targetPath } of items) {
    const renamed = new File([file], targetPath, { type: file.type });
    form.append('file', renamed);
  }
  return request<DirentBatchResult>(
    `/dirents?path=${scopeRoot(scope)}`,
    { method: 'POST', body: form, isForm: true },
  );
}

export async function createFolder(scope: DirentScope, folderPath: string): Promise<void> {
  const cleaned = folderPath.replace(/^\/+|\/+$/g, '');
  const placeholder = new File([''], `${cleaned}/.keep`, { type: 'text/plain' });
  const form = new FormData();
  form.append('file', placeholder);
  await request(
    `/dirents?path=${scopeRoot(scope)}`,
    { method: 'POST', body: form, isForm: true },
  );
}

export async function moveDirents(
  scope: DirentScope,
  sources: string[],
  destination: string,
  newName?: string,
): Promise<DirentBatchResult> {
  const root = scopeRoot(scope);
  const body: BackendDirentBatchOp = {
    op: 'move',
    sources: sources.map((s) => `${root}/${s}`),
    destination: destination ? `${root}/${destination}` : root,
    new_name: newName ?? null,
  };
  return request<DirentBatchResult>('/dirents', { method: 'PATCH', body });
}

export async function copyDirents(
  srcScope: DirentScope,
  dstScope: DirentScope,
  sources: string[],
  destination: string,
): Promise<DirentBatchResult> {
  const srcRoot = scopeRoot(srcScope);
  const dstRoot = scopeRoot(dstScope);
  const body: BackendDirentBatchOp = {
    op: 'copy',
    sources: sources.map((s) => `${srcRoot}/${s}`),
    destination: destination ? `${dstRoot}/${destination}` : dstRoot,
  };
  return request<DirentBatchResult>('/dirents', { method: 'PATCH', body });
}

export async function deleteDirent(scope: DirentScope, relativePath: string): Promise<void> {
  const globalPath = `${scopeRoot(scope)}/${relativePath}`;
  await request(`/dirents/${encodePath(globalPath)}`, { method: 'DELETE' });
}

export async function downloadFile(scope: DirentScope, relativePath: string): Promise<void> {
  const globalPath = `${scopeRoot(scope)}/${relativePath}`;
  await _fetchAndTriggerDownload(globalPath);
}

/** Fetch a file by its full global path and trigger a browser download. */
export async function downloadFileByGlobalPath(globalPath: string): Promise<void> {
  await _fetchAndTriggerDownload(globalPath);
}

/** Fetch a file by its full global path and return the blob (for thumbnails etc.). */
export async function fetchFileBlob(globalPath: string): Promise<Blob> {
  const url = `${BASE_URL}/dirents/${encodePath(globalPath)}`;
  const headers = new Headers();
  const token = getToken();
  if (token) headers.set('Authorization', `Bearer ${token}`);
  const response = await fetch(url, { headers });
  if (!response.ok) {
    const raw = await response.text().catch(() => '');
    let parsed: unknown;
    try { parsed = raw ? JSON.parse(raw) : undefined; } catch { parsed = raw; }
    notifyUnauthorized(response.status, parsed);
    throw new ApiError(response.status, raw || `${response.status} ${response.statusText}`);
  }
  return response.blob();
}

async function _fetchAndTriggerDownload(globalPath: string): Promise<void> {
  const blob = await fetchFileBlob(globalPath);
  const filename = globalPath.split('/').filter(Boolean).pop() ?? 'download';
  const objectUrl = URL.createObjectURL(blob);
  try {
    const link = document.createElement('a');
    link.href = objectUrl;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
  } finally {
    URL.revokeObjectURL(objectUrl);
  }
}
