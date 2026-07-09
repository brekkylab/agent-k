import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { EmptyState } from '@/components/uiPrimitives';
import { SourceIcon } from '@/workspace/icons';
import { getKnowledgeSourceDocuments, statusCounts } from '@/workspace/knowledge';
import { getProvider } from '@/workspace/providers';
import type { KnowledgeStatus, SourceEntry, SourceProvider } from '@/workspace/types';

interface KnowledgeRecordViewProps {
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

function statusLabel(t: (key: string) => string, status: KnowledgeStatus | undefined): string {
  return t(`workspace.knowledge.status.${status ?? 'draft'}`);
}

function sourceDocumentTitle(label: string): string {
  const separator = label.indexOf(' / ');
  return separator >= 0 ? label.slice(separator + 3) : label;
}

type KnowledgeViewMode = 'records' | 'sources' | 'conflicts';

export function KnowledgeRecordView({ provider, onSelect }: KnowledgeRecordViewProps) {
  const { t } = useTranslation('files');
  const [query, setQuery] = useState('');
  const [mode, setMode] = useState<KnowledgeViewMode>('records');

  const { data: entries, isLoading } = useQuery({
    queryKey: ['ws', provider.id, ''],
    queryFn: () => provider.list({}),
  });

  const sourceDocuments = useMemo(() => getKnowledgeSourceDocuments(entries ?? []), [entries]);
  const conflictEntries = useMemo(() => (entries ?? []).filter((entry) => entry.status === 'conflict'), [entries]);

  const filteredRecords = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!entries || q === '') return entries ?? [];
    return entries.filter((entry) => {
      const haystack = [
        entry.title,
        entry.subtitle,
        entry.collection,
        ...(entry.evidenceRefs ?? []).flatMap((evidence) => [
          evidence.label,
          evidence.excerpt,
          evidence.usedFor,
        ]),
      ].join(' ').toLowerCase();
      return haystack.includes(q);
    });
  }, [entries, query]);

  const filteredSourceDocuments = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q === '') return sourceDocuments;
    return sourceDocuments.filter((document) => {
      const haystack = [
        document.evidence.label,
        document.evidence.excerpt,
        document.evidence.usedFor,
        ...document.records.map((record) => record.title),
      ].join(' ').toLowerCase();
      return haystack.includes(q);
    });
  }, [query, sourceDocuments]);

  const filteredConflicts = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q === '') return conflictEntries;
    return conflictEntries.filter((entry) =>
      [
        entry.title,
        entry.collection,
        ...(entry.evidenceRefs ?? []).map((evidence) => `${evidence.label} ${evidence.excerpt}`),
      ].join(' ').toLowerCase().includes(q),
    );
  }, [conflictEntries, query]);

  const counts = useMemo(() => statusCounts(entries ?? []), [entries]);

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
    <section className="cw-ws-knowledge" data-testid="knowledge-record-view">
      <div className="cw-ws-knowledge-head">
        <div className="cw-ws-knowledge-titleblock">
          <span>{t('workspace.knowledge.eyebrow')}</span>
          <strong>{t('workspace.knowledge.title')}</strong>
        </div>
        <div className="cw-ws-knowledge-stats" aria-label={t('workspace.knowledge.statusSummary')}>
          {(Object.keys(counts) as KnowledgeStatus[]).map((status) => (
            <span key={status} className={`cw-ws-knowledge-stat is-${status}`}>
              {statusLabel(t, status)} <strong>{counts[status]}</strong>
            </span>
          ))}
        </div>
      </div>

      <div className="cw-ws-toolbar">
        <div className="cw-ws-knowledge-tabs" aria-label={t('workspace.knowledge.views.label')}>
          {(['records', 'sources', 'conflicts'] as KnowledgeViewMode[]).map((viewMode) => (
            <button
              key={viewMode}
              type="button"
              className={mode === viewMode ? 'is-active' : ''}
              aria-pressed={mode === viewMode}
              onClick={() => setMode(viewMode)}
            >
              {t(`workspace.knowledge.views.${viewMode}`)}
            </button>
          ))}
        </div>
        <input
          className="cw-ws-search"
          placeholder={t('workspace.knowledge.search')}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>

      {mode === 'records' && filteredRecords.length === 0 ? (
        <p className="cw-loading" style={{ padding: '16px' }}>
          {t('empty.search_no_results')}
        </p>
      ) : mode === 'records' ? (
        <ul className="cw-ws-knowledge-list">
          {filteredRecords.map((entry) => {
            const status = entry.status ?? 'draft';
            const refs = entry.evidenceRefs ?? [];
            return (
              <li key={entry.id}>
                <button className="cw-ws-knowledge-row" onClick={() => onSelect(entry)}>
                  <span className={`cw-ws-knowledge-status is-${status}`}>
                    {statusLabel(t, status)}
                  </span>
                  <span className="cw-ws-knowledge-row-main">
                    <span className="cw-ws-knowledge-row-title">{entry.title}</span>
                    <span className="cw-ws-knowledge-row-subtitle">
                      {entry.collection}
                      {refs.length > 0 && (
                        <>
                          <span aria-hidden="true"> · </span>
                          {t('workspace.knowledge.sourceCount', { count: refs.length })}
                        </>
                      )}
                    </span>
                    {refs.length > 0 && (
                      <span className="cw-ws-knowledge-provenance">
                        {refs.slice(0, 3).map((ref) => (
                          <span key={ref.id}>{ref.label}</span>
                        ))}
                      </span>
                    )}
                  </span>
                  <span className="cw-ws-knowledge-row-meta">
                    {entry.confidence != null && (
                      <span>{Math.round(entry.confidence * 100)}%</span>
                    )}
                    <span>{formatDate(entry.modifiedAt)}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      ) : mode === 'sources' && filteredSourceDocuments.length === 0 ? (
        <p className="cw-loading" style={{ padding: '16px' }}>
          {t('empty.search_no_results')}
        </p>
      ) : mode === 'sources' ? (
        <ul className="cw-ws-knowledge-list">
          {filteredSourceDocuments.map((document) => {
            const sourceProvider = getProvider(document.evidence.sourceId);
            const sourceName = sourceProvider ? t(sourceProvider.nameKey) : document.evidence.sourceId;
            const title = sourceDocumentTitle(document.evidence.label);

            return (
              <li key={document.key}>
                <button
                  className="cw-ws-knowledge-source-row"
                  onClick={() => onSelect(document.sourceEntry)}
                >
                  <span className="cw-ws-knowledge-source-origin">
                    <SourceIcon sourceId={document.evidence.sourceId} size={18} />
                    <span>{sourceName}</span>
                  </span>
                  <span className="cw-ws-knowledge-source-main">
                    <span className="cw-ws-knowledge-source-label">{title}</span>
                    <span className="cw-ws-knowledge-source-excerpt">{document.evidence.excerpt}</span>
                    <span className="cw-ws-knowledge-provenance">
                      {document.records.map((record) => (
                        <span key={record.id}>{record.title}</span>
                      ))}
                    </span>
                  </span>
                  <span className="cw-ws-knowledge-source-side">
                    <strong>{t('workspace.knowledge.recordCount', { count: document.records.length })}</strong>
                    <span>{document.records.map((record) => statusLabel(t, record.status)).join(' · ')}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      ) : filteredConflicts.length === 0 ? (
        <p className="cw-loading" style={{ padding: '16px' }}>
          {t('empty.search_no_results')}
        </p>
      ) : (
        <ul className="cw-ws-knowledge-list">
          {filteredConflicts.map((entry) => (
            <li key={entry.id}>
              <button className="cw-ws-knowledge-conflict-row" onClick={() => onSelect(entry)}>
                <span className="cw-ws-knowledge-status is-conflict">
                  {statusLabel(t, 'conflict')}
                </span>
                <span className="cw-ws-knowledge-row-main">
                  <span className="cw-ws-knowledge-row-title">{entry.title}</span>
                  <span className="cw-ws-knowledge-row-subtitle">{entry.collection}</span>
                  <span className="cw-ws-knowledge-conflict-evidence">
                    {(entry.evidenceRefs ?? []).map((evidence) => (
                      <span key={evidence.id}>
                        <strong>{evidence.label}</strong>
                        {evidence.excerpt}
                      </span>
                    ))}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
