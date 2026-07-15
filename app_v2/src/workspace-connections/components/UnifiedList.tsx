import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import dayjs from 'dayjs';
import { SourceIcon } from '@/workspace-connections/icons';
import { useProviders } from '@/workspace-connections/hooks/useProviders';
import type { SourceEntry } from '@/workspace-connections/types';

interface UnifiedListProps {
  entries: SourceEntry[];
  onSelect: (entry: SourceEntry) => void;
  loading?: boolean;
}

export function UnifiedList({ entries, onSelect, loading = false }: UnifiedListProps) {
  const { t } = useTranslation('files');
  const providers = useProviders();
  const [query, setQuery] = useState('');

  const filtered = query.trim()
    ? entries.filter((e) =>
        e.title.toLowerCase().includes(query.toLowerCase()) ||
        (e.subtitle ?? '').toLowerCase().includes(query.toLowerCase()),
      )
    : entries;

  return (
    <>
      <div className="cw-ws-toolbar">
        <input
          className="cw-ws-search"
          placeholder={t('workspace.searchAll')}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      {loading && (
        <p className="cw-loading" style={{ padding: '16px' }}>{t('workspace.loading')}</p>
      )}

      {!loading && filtered.length === 0 && (
        <p className="cw-loading" style={{ padding: '16px' }}>
          {t('empty.search_no_results')}
        </p>
      )}

      <div className="cw-ws-list">
        {filtered.map((entry) => {
          const provider = providers.find((p) => p.id === entry.sourceId);
          const nameKey = provider?.nameKey ?? `workspace.src.${entry.sourceId}`;
          const meta = dayjs(entry.modifiedAt).format('MM-DD');

          return (
            <button
              key={entry.id}
              className="cw-ws-row"
              onClick={() => onSelect(entry)}
            >
              <SourceIcon sourceId={provider?.type ?? entry.sourceId} size={18} />
              <span className="cw-ws-row-source">{provider?.label ?? t(nameKey)}</span>
              <span className="cw-ws-row-title" data-testid="unified-row-title">
                {entry.title}
              </span>
              <span className="cw-ws-row-meta">{meta}</span>
            </button>
          );
        })}
      </div>
    </>
  );
}
