import { useEffect, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { EmptyState } from '@/components/uiPrimitives';
import type { SourceEntry, SourceProvider } from '@/workspace/types';

interface NotionPageViewProps {
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

function newestFirst(a: SourceEntry, b: SourceEntry) {
  return b.modifiedAt.localeCompare(a.modifiedAt);
}

function parentKey(parentId: string | null | undefined) {
  return parentId ?? '__root__';
}

export function NotionPageView({ provider, onSelect }: NotionPageViewProps) {
  const { t } = useTranslation('files');
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const { data: pages, isLoading } = useQuery({
    queryKey: ['ws', provider.id, 'pages'],
    queryFn: () => provider.list({}),
  });

  const pageList = useMemo(
    () => (pages ?? []).filter((entry) => entry.kind === 'page'),
    [pages],
  );

  const pagesByParent = useMemo(() => {
    const map = new Map<string, SourceEntry[]>();
    for (const page of pageList) {
      const key = parentKey(page.parentId);
      map.set(key, [...(map.get(key) ?? []), page]);
    }
    for (const [key, children] of map) {
      map.set(key, [...children].sort((a, b) => a.title.localeCompare(b.title)));
    }
    return map;
  }, [pageList]);

  const roots = useMemo(
    () => [...(pagesByParent.get(parentKey(null)) ?? [])],
    [pagesByParent],
  );

  useEffect(() => {
    if (selectedId || roots.length === 0) return;
    setSelectedId([...roots].sort(newestFirst)[0].id);
  }, [roots, selectedId]);

  const selected = pageList.find((page) => page.id === selectedId) ?? roots[0] ?? null;

  useEffect(() => {
    if (!selected) return;
    onSelect(selected);
  }, [onSelect, selected]);

  function togglePage(id: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  function selectPage(page: SourceEntry) {
    setSelectedId(page.id);
  }

  function renderTree(page: SourceEntry, depth: number) {
    const children = pagesByParent.get(page.id) ?? [];
    const isExpanded = expanded.has(page.id);
    const isSelected = selected?.id === page.id;

    return (
      <li key={page.id}>
        <div
          className={`cw-ws-notion-tree-row${isSelected ? ' is-active' : ''}`}
          style={{ paddingLeft: 8 + depth * 14 }}
        >
          {children.length > 0 ? (
            <button
              type="button"
              className="cw-ws-notion-toggle"
              aria-label={`${isExpanded ? 'Collapse' : 'Expand'} ${page.title}`}
              onClick={() => togglePage(page.id)}
            >
              {isExpanded ? '⌄' : '›'}
            </button>
          ) : (
            <span className="cw-ws-notion-toggle-placeholder" />
          )}
          <button
            type="button"
            className="cw-ws-notion-page-button"
            onClick={() => selectPage(page)}
          >
            <span aria-hidden="true">{page.emoji ?? '📄'}</span>
            <span className="cw-ws-notion-page-text">
              <span className="cw-ws-notion-page-title">{page.title}</span>
              {page.subtitle && <span className="cw-ws-notion-page-subtitle">{page.subtitle}</span>}
            </span>
            <span className="cw-ws-notion-page-meta">{formatDate(page.modifiedAt)}</span>
          </button>
        </div>
        {isExpanded && children.length > 0 && (
          <ul className="cw-ws-notion-tree-list">
            {children.map((child) => renderTree(child, depth + 1))}
          </ul>
        )}
      </li>
    );
  }

  if (isLoading) {
    return (
      <p className="cw-loading" style={{ padding: '16px 24px' }}>
        {t('workspace.loading')}
      </p>
    );
  }

  if (pageList.length === 0 || !selected) {
    return <EmptyState title={t('workspace.emptySource')} body={t('workspace.emptySourceHint')} />;
  }

  return (
    <div className="cw-ws-notion">
      <nav className="cw-ws-notion-sidebar" aria-label="Notion pages">
        <div className="cw-ws-notion-sidebar-head">
          <span>{t(provider.nameKey)}</span>
          <strong>{pageList.length}</strong>
        </div>
        <ul className="cw-ws-notion-tree-list" data-testid="notion-page-tree">
          {roots.map((root) => renderTree(root, 0))}
        </ul>
      </nav>
    </div>
  );
}
