// WebDAV client wrapper — blob-based, header-authenticated.
// The token can rotate after silent re-login, so a fresh client is created
// per call rather than cached as a module singleton.
// NO token in any URL — the only auth is Authorization: Bearer in headers.

import { AuthType, createClient, type FileStat, type WebDAVClient } from 'webdav';
import { BASE_URL, getToken, reauthOnce } from './client';
import { getWorkspaceId } from '../stores/workspace';

function workspaceClient(): WebDAVClient {
  return createClient(`${BASE_URL}/workspaces/${getWorkspaceId()}/files`, {
    authType: AuthType.None,
    headers: { Authorization: `Bearer ${getToken() ?? ''}` },
  });
}

// Inspect the error thrown by the webdav library for a 401 status.
function is401(err: unknown): boolean {
  if (typeof err !== 'object' || err === null) return false;
  const e = err as Record<string, unknown>;
  if (typeof e['status'] === 'number') return e['status'] === 401;
  const resp = e['response'] as Record<string, unknown> | undefined;
  if (resp && typeof resp['status'] === 'number') return resp['status'] === 401;
  return false;
}

// Retry the op once after reauth if a 401 is detected.
async function withReauth<T>(op: () => Promise<T>): Promise<T> {
  try {
    return await op();
  } catch (err) {
    if (is401(err)) {
      const ok = await reauthOnce();
      if (ok) return op();
    }
    throw err;
  }
}

export async function listDirectory(path?: string): Promise<FileStat[]> {
  return withReauth(async () => {
    const result = await workspaceClient().getDirectoryContents(path ?? '/');
    return Array.isArray(result) ? result : (result as { data: FileStat[] }).data;
  });
}

export async function getFileBlob(path: string): Promise<Blob> {
  return withReauth(async () => {
    const buf = await workspaceClient().getFileContents(path, { format: 'binary' });
    return new Blob([buf as ArrayBuffer]);
  });
}

export async function putFile(path: string, data: Blob | ArrayBuffer): Promise<void> {
  return withReauth(async () => {
    const buf = data instanceof Blob ? await data.arrayBuffer() : data;
    await workspaceClient().putFileContents(path, buf);
  });
}

export async function deleteEntry(path: string): Promise<void> {
  return withReauth(() => workspaceClient().deleteFile(path));
}

export async function createDirectory(path: string): Promise<void> {
  return withReauth(() => workspaceClient().createDirectory(path));
}
