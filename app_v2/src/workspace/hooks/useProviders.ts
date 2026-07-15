// Mount-aware provider resolution. The static catalog (providers.ts) is overlaid
// with real providers for each of the workspace's active mounts, fetched via
// React Query. Components consume these hooks instead of importing the static
// PROVIDERS directly; the only out-of-React consumer (the router loader guard)
// uses the synchronous getProviderMeta on the catalog.

import { useMemo } from 'react';
import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { listMounts, type MountResponse } from '@/api/mounts';
import { buildProviders } from '../providers';
import type { SourceProvider } from '../types';

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
