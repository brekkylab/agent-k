import { createFileRoute, redirect, useParams } from '@tanstack/react-router';
import { getProviderMeta } from '@/workspace/providers';
import { useProvider } from '@/workspace/hooks/useProviders';
import { FileBrowserView } from '@/workspace/components/FileBrowserView';
import { ItemListView } from '@/workspace/components/ItemListView';
import { KnowledgeRecordView } from '@/workspace/components/KnowledgeRecordView';
import { NotionPageView } from '@/workspace/components/NotionPageView';
import { ThreadListView } from '@/workspace/components/ThreadListView';
import { useWorkspaceSelection } from '@/workspace/components/WorkspaceShell';

export const Route = createFileRoute('/workspace/$sourceId')({
  beforeLoad: ({ params }) => {
    // Catalog membership only — synchronous, mount-independent. Runs outside
    // React so it can't use the mount-aware hooks.
    if (!getProviderMeta(params.sourceId)) throw redirect({ to: '/workspace' });
  },
  component: SourcePage,
});

export function SourcePage() {
  const { sourceId } = useParams({ from: '/workspace/$sourceId' });
  const { onSelect } = useWorkspaceSelection();
  // Mount-aware resolution: undefined until the mounts query resolves, even
  // though beforeLoad already confirmed the id is a valid catalog source.
  const provider = useProvider(sourceId);

  if (!provider) return null;

  return (
    provider.kind === 'files' ? (
      <FileBrowserView provider={provider} onSelect={onSelect} />
    ) : provider.kind === 'items' ? (
      <ItemListView provider={provider} onSelect={onSelect} />
    ) : provider.kind === 'pages' ? (
      <NotionPageView provider={provider} onSelect={onSelect} />
    ) : provider.kind === 'records' ? (
      <KnowledgeRecordView provider={provider} onSelect={onSelect} />
    ) : (
      <ThreadListView provider={provider} onSelect={onSelect} />
    )
  );
}
