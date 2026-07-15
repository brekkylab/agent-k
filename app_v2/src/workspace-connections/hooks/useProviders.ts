// Mount-aware provider resolution. The static catalog (providers.ts) is overlaid
// with real providers for each of the workspace's active mounts, fetched via
// React Query. Components consume these hooks instead of importing the static
// PROVIDERS directly; the only out-of-React consumer (the router loader guard)
// uses the synchronous getProviderMeta on the catalog.

import { useMemo } from 'react';
import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { listMounts, type MountResponse } from '@/api/mounts';
import { buildProviders, PROVIDERS, MOUNT_BACKED_TYPES } from '../providers';
import type { SourceProvider, SourceType } from '../types';

export function useMounts(): UseQueryResult<MountResponse[]> {
  return useQuery({
    queryKey: ['workspace', 'mounts'],
    queryFn: listMounts,
    staleTime: 30_000,
  });
}

// Resolved provider list = static catalog + real-mount overlay. Memoized on the
// mounts reference (React Query keeps `data` stable between refetches) so
// provider identity is stable across renders and doesn't churn child query keys.
export function useProviders(): SourceProvider[] {
  const { data: mounts } = useMounts();
  return useMemo(() => buildProviders(mounts ?? []), [mounts]);
}

export function useProvider(id: string | undefined): SourceProvider | undefined {
  const providers = useProviders();
  return useMemo(
    () => (id == null ? undefined : providers.find((p) => p.id === id)),
    [providers, id],
  );
}

// Catalog TYPES the user could connect but hasn't — the discovery-hint pool.
// Mount-backed types (s3/notion) qualify when they have zero mounts; the
// mock-connectable catalog entries qualify when not in `connectedIds`. Static
// mocks that are always "connected" (gdrive/jira/…) and `local` are excluded.
export function useUnconnectedCatalog(connectedIds: Set<string>): SourceProvider[] {
  const { data: mounts } = useMounts();
  return useMemo(() => {
    const mountedTypes = new Set<SourceType>(
      (mounts ?? []).map((m) => m.provider.type as SourceType),
    );
    return PROVIDERS.filter((p) => {
      if (p.connected) return false;
      if (MOUNT_BACKED_TYPES.has(p.type)) return !mountedTypes.has(p.type);
      return !connectedIds.has(p.id);
    });
  }, [mounts, connectedIds]);
}
