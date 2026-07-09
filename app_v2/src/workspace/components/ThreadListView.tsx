import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { EmptyState } from '@/components/uiPrimitives';
import type { SourceEntry, SourceProvider } from '@/workspace/types';

interface ThreadListViewProps {
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
 * ThreadListView — renders a flat list for 'threads' kind providers (Gmail, Slack, etc.).
 * Each row shows a bold sender line (subtitle), thread title, and date.
 * The sender element carries data-testid="thread-sender-{id}" for test targeting.
 * Row click calls onSelect with the entry.
 */
export function ThreadListView({ provider, onSelect }: ThreadListViewProps) {
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
      {entries.map((entry) => (
        <li key={entry.id}>
          <button className="cw-ws-row" onClick={() => onSelect(entry)}>
            <span className="cw-ws-row-body" style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: 2 }}>
              {/* Bold sender — subtitle carries the full "Name <email>" string */}
              {entry.subtitle && (
                <span
                  className="cw-ws-row-sender"
                  data-testid={`thread-sender-${entry.id}`}
                >
                  {entry.subtitle}
                </span>
              )}
              <span className="cw-ws-row-subtitle">{entry.title}</span>
            </span>
            <span className="cw-ws-row-meta">{formatDate(entry.modifiedAt)}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}
