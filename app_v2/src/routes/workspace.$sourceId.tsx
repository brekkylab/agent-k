import { createFileRoute, redirect, useParams } from '@tanstack/react-router';
import { getProvider } from '@/workspace/providers';
import { FileBrowserView } from '@/workspace/components/FileBrowserView';
import { ItemListView } from '@/workspace/components/ItemListView';
import { KnowledgeRecordView } from '@/workspace/components/KnowledgeRecordView';
import { NotionPageView } from '@/workspace/components/NotionPageView';
import { ThreadListView } from '@/workspace/components/ThreadListView';
import { useWorkspaceSelection } from '@/workspace/components/WorkspaceShell';

export const Route = createFileRoute('/workspace/$sourceId')({
  beforeLoad: ({ params }) => {
    if (!getProvider(params.sourceId)) throw redirect({ to: '/workspace' });
  },
  component: SourcePage,
});

export function SourcePage() {
  const { sourceId } = useParams({ from: '/workspace/$sourceId' });
  const { onSelect } = useWorkspaceSelection();
  // beforeLoad guarantees the provider exists, so the non-null assertion is safe.
  const provider = getProvider(sourceId)!;

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
