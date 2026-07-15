import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { EmptyState } from '@/components/uiPrimitives';
import type { SourceEntry, SourceProvider } from '@/workspace-connections/types';

interface FileBrowserViewProps {
  provider: SourceProvider;
  onSelect: (entry: SourceEntry) => void;
}

/** A breadcrumb step: the display title and the path it points to. */
interface BreadcrumbStep {
  label: string;
  path: string;
}

/** Format a byte count as a human-readable string (B / KB / MB). */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
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
 * FileBrowserView — renders a folder-tree view for 'files' kind providers.
 * Supports breadcrumb navigation; folders descend deeper, files call onSelect.
 * CRITICAL: never calls onSelect for folder entries, never renders the current
 * folder itself as a row (entry.path === ctx.path is filtered out).
 *
 * Breadcrumbs track folder display titles (not raw path segments) so the user
 * always sees the human-readable name they clicked.
 */
export function FileBrowserView({ provider, onSelect }: FileBrowserViewProps) {
  const { t } = useTranslation('files');

  // crumbs is the navigation stack: each entry holds the path AND the user-facing
  // title of the folder, so breadcrumbs display folder names not raw path tokens.
  const [crumbs, setCrumbs] = useState<BreadcrumbStep[]>([]);

  // The current folder path is the last crumb's path, or '' for root.
  const currentPath = crumbs.length > 0 ? crumbs[crumbs.length - 1].path : '';

  const { data: entries, isLoading } = useQuery({
    queryKey: ['ws', provider.id, currentPath],
    queryFn: () => provider.list({ path: currentPath }),
  });

  // Filter: exclude the current folder entry itself, then sort folders first.
  const rows = (entries ?? [])
    .filter((e) => e.path !== currentPath)
    .sort((a, b) => {
      if (a.kind === 'folder' && b.kind !== 'folder') return -1;
      if (a.kind !== 'folder' && b.kind === 'folder') return 1;
      return a.title.localeCompare(b.title);
    });

  function handleRowClick(entry: SourceEntry) {
    if (entry.kind === 'folder') {
      // Navigate deeper — push the folder's display title onto the crumb stack.
      // Never call onSelect for folders.
      setCrumbs((prev) => [...prev, { label: entry.title, path: entry.path ?? '' }]);
    } else {
      onSelect(entry);
    }
  }

  function navigateToCrumb(index: number) {
    // index -1 means root; otherwise slice to keep crumbs up to and including index.
    if (index < 0) {
      setCrumbs([]);
    } else {
      setCrumbs((prev) => prev.slice(0, index + 1));
    }
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      {/* Breadcrumb */}
      <div className="cw-ws-breadcrumb">
        <button
          className="cw-ws-breadcrumb-crumb"
          onClick={() => navigateToCrumb(-1)}
        >
          {provider.label ?? t(provider.nameKey)}
        </button>
        {crumbs.map((crumb, i) => (
          <span key={crumb.path} style={{ display: 'contents' }}>
            <span className="cw-ws-breadcrumb-sep">/</span>
            {i === crumbs.length - 1 ? (
              <span className="cw-ws-breadcrumb-cur">{crumb.label}</span>
            ) : (
              <button
                className="cw-ws-breadcrumb-crumb"
                onClick={() => navigateToCrumb(i)}
              >
                {crumb.label}
              </button>
            )}
          </span>
        ))}
      </div>

      {/* Content */}
      {isLoading ? (
        <p className="cw-loading" style={{ padding: '16px 24px' }}>
          {t('workspace.loading')}
        </p>
      ) : rows.length === 0 ? (
        <EmptyState title={t('workspace.folderEmpty')} body={t('workspace.folderEmptyHint')} />
      ) : (
        <ul className="cw-ws-list" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
          {rows.map((entry) => (
            <li key={entry.id}>
              <button className="cw-ws-row" onClick={() => handleRowClick(entry)}>
                {/* Kind icon: simple text indicator */}
                <span className="cw-ws-row-source" aria-hidden="true">
                  {entry.kind === 'folder' ? '📁' : '📄'}
                </span>
                <span className="cw-ws-row-title">{entry.title}</span>
                {entry.size != null && (
                  <span className="cw-ws-row-meta">{formatBytes(entry.size)}</span>
                )}
                <span className="cw-ws-row-meta">{formatDate(entry.modifiedAt)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
