// Workspace REST resource — used by bootstrap to resolve the caller's default
// workspace. Distinct from api/workspace.ts, which is the WebDAV *file* client.
// Every user gets one default workspace at signup (its id mirrors the user id),
// so bootstrap needs only this single fetch — no list/create dance.

import { request } from './client';
import type { WorkspaceResponse } from './types';

export async function getMyWorkspace(): Promise<WorkspaceResponse> {
  return request<WorkspaceResponse>('/me/workspace');
}
