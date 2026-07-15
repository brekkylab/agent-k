// External-provider mount CRUD for a workspace.
// Mirrors backend_v2 `router/mount.rs`: GET/POST /workspaces/{wid}/mounts and
// DELETE /workspaces/{wid}/mounts/{id}. Auth is the standard Bearer header via
// request() (reauth-once included).

import { request } from './client';
import { getWorkspaceId } from '@/stores/workspace';

// Non-secret provider view returned in responses. The api_key is never echoed
// back for Notion (backend serializes it as `{ type: "notion" }`), so the list
// can show that a mount exists but never re-read its key.
export type ProviderInfo =
  | { type: 'notion' }
  | { type: 's3'; bucket: string; region: string; endpoint?: string; key_prefix?: string };

export interface MountResponse {
  id: string;
  workspace_id: string;
  prefix: string;
  provider: ProviderInfo;
  // Optional user-chosen display name (distinct from `prefix`). Null/absent for
  // older mounts; the UI falls back to the prefix.
  label?: string | null;
  created_at: string;
  updated_at: string;
}

// Provider config as supplied when creating a mount (carries secrets).
export type ProviderSpec =
  | { type: 'notion'; api_key: string }
  | {
      type: 's3';
      bucket: string;
      region?: string;
      access_key_id: string;
      secret_access_key: string;
      endpoint?: string;
      key_prefix?: string;
    };

export async function listMounts(): Promise<MountResponse[]> {
  const result = await request<{ items: MountResponse[] }>(
    `/workspaces/${getWorkspaceId()}/mounts`,
  );
  return result.items;
}

export async function createMount(input: {
  prefix: string;
  provider: ProviderSpec;
  label?: string;
}): Promise<MountResponse> {
  return request<MountResponse>(`/workspaces/${getWorkspaceId()}/mounts`, {
    method: 'POST',
    body: { prefix: input.prefix, provider: input.provider, label: input.label },
  });
}

export async function deleteMount(mountId: string): Promise<void> {
  await request<void>(`/workspaces/${getWorkspaceId()}/mounts/${mountId}`, {
    method: 'DELETE',
  });
}
