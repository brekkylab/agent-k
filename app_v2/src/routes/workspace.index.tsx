import { createFileRoute } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { allRecent } from '@/workspace/providers';
import { UnifiedList } from '@/workspace/components/UnifiedList';
import { useWorkspaceSelection } from '@/workspace/components/WorkspaceShell';

export const Route = createFileRoute('/workspace/')({
  component: WorkspaceIndexPage,
});

function WorkspaceIndexPage() {
  const { onSelect } = useWorkspaceSelection();
  const { data: entries = [], isLoading } = useQuery({
    queryKey: ['ws', 'all'],
    queryFn: allRecent,
  });

  return <UnifiedList entries={entries} onSelect={onSelect} loading={isLoading} />;
}
