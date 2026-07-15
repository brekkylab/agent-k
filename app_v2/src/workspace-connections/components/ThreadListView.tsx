import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { EmptyState } from '@/components/uiPrimitives';
import { SourceIcon } from '@/workspace-connections/icons';
import type { SourceEntry, SourceProvider } from '@/workspace-connections/types';

interface ThreadListViewProps {
  provider: SourceProvider;
  onSelect: (entry: SourceEntry) => void;
}

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return iso;
  }
}

function threadLabel(provider: SourceProvider): string {
  if (provider.type === 'gmail') return 'Email';
  if (provider.type === 'slack') return 'Channel';
  return 'Thread';
}

function avatarText(entry: SourceEntry, provider: SourceProvider): string {
  if (provider.type === 'slack') return '#';
  const seed = entry.subtitle ?? entry.title;
  return seed.trim().slice(0, 1).toUpperCase();
}

function isHighSignal(entry: SourceEntry): boolean {
  return /긴급|보안|인시던트|alert|incident|security/i.test(entry.title);
}

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
    <ul className="cw-ws-list cw-ws-thread-list">
      {entries.map((entry) => (
        <li key={entry.id}>
          <button
            className={`cw-ws-thread-row${isHighSignal(entry) ? ' is-high-signal' : ''}`}
            onClick={() => onSelect(entry)}
          >
            <span className="cw-ws-thread-avatar" aria-hidden="true">
              <span className="cw-ws-thread-avatar-mark">{avatarText(entry, provider)}</span>
              <span className="cw-ws-thread-source-icon">
                <SourceIcon sourceId={provider.type} size={14} />
              </span>
            </span>
            <span className="cw-ws-thread-content">
              <span className="cw-ws-thread-topline">
                {entry.subtitle && (
                  <span
                    className="cw-ws-thread-sender"
                    data-testid={`thread-sender-${entry.id}`}
                  >
                    {entry.subtitle}
                  </span>
                )}
                <span className="cw-ws-thread-date">{formatDate(entry.modifiedAt)}</span>
              </span>
              <span className="cw-ws-thread-title">{entry.title}</span>
              <span className="cw-ws-thread-foot">
                <span className="cw-ws-thread-pill">{threadLabel(provider)}</span>
                {isHighSignal(entry) && <span className="cw-ws-thread-priority">Needs review</span>}
              </span>
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}
