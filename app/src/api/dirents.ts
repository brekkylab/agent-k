import { ApiError, getBaseUrl, getToken, request } from './client';
import type { BackendDirent, BackendUploadResponse } from './backend-types';
import { toFileAsset } from './transformers';
import type { FileAsset } from '@/domain/types';

export type UploadResult = BackendUploadResponse;

function encodePath(path: string): string {
  return path.split('/').map(encodeURIComponent).join('/');
}

export async function listDirents(projectId: string, projectName: string, recursive = true): Promise<FileAsset[]> {
  const res = await request<{ entries: BackendDirent[] }>(
    `/projects/${projectId}/dirents?recursive=${recursive}`,
  );
  return res.entries.map((e) => toFileAsset(e, projectId, projectName));
}

// Raw entries — preferred for tree navigation in the Files page where
// the unmodified backend path is needed for both display and mutations.
export async function listDirentsRaw(projectId: string, recursive = true): Promise<BackendDirent[]> {
  const res = await request<{ entries: BackendDirent[] }>(
    `/projects/${projectId}/dirents?recursive=${recursive}`,
  );
  return res.entries;
}

export async function uploadFile(projectId: string, file: File, targetPath: string): Promise<UploadResult> {
  return uploadFiles(projectId, [{ file, targetPath }]);
}

// Upload many files in a single multipart request. Backend handles each
// file independently and returns { succeeded, failed } so callers can show
// partial-failure UI without parsing per-file errors out of a single message.
export async function uploadFiles(
  projectId: string,
  items: Array<{ file: File; targetPath: string }>,
): Promise<UploadResult> {
  const form = new FormData();
  for (const { file, targetPath } of items) {
    const renamed = new File([file], targetPath, { type: file.type });
    form.append('file', renamed);
  }
  return request<UploadResult>(`/projects/${projectId}/dirents`, {
    method: 'POST',
    body: form,
    isForm: true,
  });
}

export async function createFolder(projectId: string, folderPath: string): Promise<void> {
  const cleaned = folderPath.replace(/^\/+|\/+$/g, '');
  const placeholder = new File([''], `${cleaned}/.keep`, { type: 'text/plain' });
  const form = new FormData();
  form.append('file', placeholder);
  await request(`/projects/${projectId}/dirents`, { method: 'POST', body: form, isForm: true });
}

export async function deleteDirent(projectId: string, path: string): Promise<void> {
  await request(`/projects/${projectId}/dirents/${encodePath(path)}`, { method: 'DELETE' });
}

// Authenticated download. <a href> can't carry the Authorization header, so
// we fetch the blob ourselves and trigger a download via a synthetic anchor.
export async function downloadFile(projectId: string, path: string): Promise<void> {
  const url = `${getBaseUrl()}/projects/${projectId}/dirents/${encodePath(path)}`;
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
