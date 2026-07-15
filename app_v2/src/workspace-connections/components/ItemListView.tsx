import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { EmptyState } from '@/components/uiPrimitives';
import type { SourceEntry, SourceProvider } from '@/workspace-connections/types';

interface ItemListViewProps {
  provider: SourceProvider;
  onSelect: (entry: SourceEntry) => void;
}

/** Format an ISO date string as a short locale date. */
function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return iso;
  }
}

/**
 * ItemListView — renders a flat list for 'items' kind providers (Confluence, Jira, etc.).
 * Each row shows a space/status chip (first token of subtitle), title, and date.
 * Row click calls onSelect with the entry.
 */
export function ItemListView({ provider, onSelect }: ItemListViewProps) {
  const { t } = useTranslation('files');

  const { data: entries, isLoading } = useQuery({
    queryKey: ['ws', provider.id, ''],
    queryFn: () => provider.list({}),
  });

  if (isLoading) {
    return (
      <p className="cw-loading" style={{ padding: '16px 24px' }}>
        {t('workspace.loading')}
      </p>
    );
  }

  if (!entries || entries.length === 0) {
    return <EmptyState title={t('workspace.emptySource')} body={t('workspace.emptySourceHint')} />;
  }

  return (
    <ul className="cw-ws-list" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
      {entries.map((entry) => {
        // First whitespace-delimited token of subtitle becomes the chip label.
        const chip = entry.subtitle ? entry.subtitle.split(' ')[0] : null;
        return (
          <li key={entry.id}>
            <button className="cw-ws-row" onClick={() => onSelect(entry)}>
              {chip && (
                <span className="cw-ws-item-chip">{chip}</span>
              )}
              <span className="cw-ws-row-title">{entry.title}</span>
              <span className="cw-ws-row-meta">{formatDate(entry.modifiedAt)}</span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
