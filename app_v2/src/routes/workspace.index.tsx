import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { recentAcross } from '@/workspace/providers';
import { useProviders } from '@/workspace/hooks/useProviders';
import { UnifiedList } from '@/workspace/components/UnifiedList';
import { useWorkspaceSelection } from '@/workspace/components/WorkspaceShell';

export const Route = createFileRoute('/workspace/')({
  component: WorkspaceIndexPage,
});

function WorkspaceIndexPage() {
  const { onSelect } = useWorkspaceSelection();
  const providers = useProviders();
  // Key on the connected provider ids so recents refetch when a mount flips a
  // source from mock to real (or connects a new one).
  const connectedIds = providers.filter((p) => p.connected).map((p) => p.id);
  const { data: entries = [], isLoading } = useQuery({
    queryKey: ['ws', 'all', connectedIds],
    queryFn: () => recentAcross(providers),
  });

  return <UnifiedList entries={entries} onSelect={onSelect} loading={isLoading} />;
}
