import { ApiError, getBaseUrl, getToken, request } from './client';
import type {
  BackendDirent,
  BackendDirentBatchOp,
  BackendDirentBatchResult,
} from './backend-types';
import { toFileAsset } from './transformers';
import type { FileAsset } from '@/domain/types';

export type DirentBatchResult = BackendDirentBatchResult;

function encodePath(path: string): string {
  return path.split('/').map(encodeURIComponent).join('/');
}

// Backend returns the resolved project UUID alongside entries; we pass that
// real id (not the slug from the URL) into FileAsset so its `projectId` keeps
// its UUID-only contract.
interface BackendListResponse {
  project_id: string;
  entries: BackendDirent[];
}

export async function listDirents(projectRef: string, projectName: string, recursive = true): Promise<FileAsset[]> {
  const res = await request<BackendListResponse>(
    `/projects/${projectRef}/dirents?recursive=${recursive}`,
  );
  return res.entries.map((e) => toFileAsset(e, res.project_id, projectName));
}

// Raw entries — preferred for tree navigation in the Files page where
// the unmodified backend path is needed for both display and mutations.
export async function listDirentsRaw(projectRef: string, recursive = true): Promise<BackendDirent[]> {
  const res = await request<BackendListResponse>(
    `/projects/${projectRef}/dirents?recursive=${recursive}`,
  );
  return res.entries;
}

export async function uploadFile(projectRef: string, file: File, targetPath: string): Promise<DirentBatchResult> {
  return uploadFiles(projectRef, [{ file, targetPath }]);
}

// Upload many files in a single multipart request. Backend handles each
// file independently and returns { succeeded, failed } so callers can show
// partial-failure UI without parsing per-file errors out of a single message.
export async function uploadFiles(
  projectRef: string,
  items: Array<{ file: File; targetPath: string }>,
): Promise<DirentBatchResult> {
  const form = new FormData();
  for (const { file, targetPath } of items) {
    const renamed = new File([file], targetPath, { type: file.type });
    form.append('file', renamed);
  }
  return request<DirentBatchResult>(`/projects/${projectRef}/dirents`, {
    method: 'POST',
    body: form,
    isForm: true,
  });
}

export async function createFolder(projectRef: string, folderPath: string): Promise<void> {
  const cleaned = folderPath.replace(/^\/+|\/+$/g, '');
  const placeholder = new File([''], `${cleaned}/.keep`, { type: 'text/plain' });
  const form = new FormData();
  form.append('file', placeholder);
  await request(`/projects/${projectRef}/dirents`, { method: 'POST', body: form, isForm: true });
}

// Collection-level batch op (move/copy). Rename is just a single-source move
// with new_name set.
export async function moveDirents(
  projectRef: string,
  sources: string[],
  destination: string,
  newName?: string,
): Promise<DirentBatchResult> {
  const body: BackendDirentBatchOp = {
    op: 'move',
    sources,
    destination,
    new_name: newName ?? null,
  };
  return request<DirentBatchResult>(`/projects/${projectRef}/dirents`, {
    method: 'PATCH',
    body,
  });
}

export async function copyDirents(
  projectRef: string,
  sources: string[],
  destination: string,
): Promise<DirentBatchResult> {
  const body: BackendDirentBatchOp = { op: 'copy', sources, destination };
  return request<DirentBatchResult>(`/projects/${projectRef}/dirents`, {
    method: 'PATCH',
    body,
  });
}

export async function deleteDirent(projectRef: string, path: string): Promise<void> {
  await request(`/projects/${projectRef}/dirents/${encodePath(path)}`, { method: 'DELETE' });
}

// Authenticated download. <a href> can't carry the Authorization header, so
// we fetch the blob ourselves and trigger a download via a synthetic anchor.
export async function downloadFile(projectRef: string, path: string): Promise<void> {
  const url = `${getBaseUrl()}/projects/${projectRef}/dirents/${encodePath(path)}`;
  const headers = new Headers();
  const token = getToken();
  if (token) headers.set('Authorization', `Bearer ${token}`);

  const response = await fetch(url, { headers });
  if (!response.ok) {
    const raw = await response.text().catch(() => '');
    throw new ApiError(response.status, raw || `${response.status} ${response.statusText}`);
  }

  const blob = await response.blob();
  const filename = path.split('/').filter(Boolean).pop() ?? 'download';
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
