// SharedFilesBrowser — a Files-page-style browser that takes over the session
// sidebar. Opened from the sidebar's "browse" button; lets the user navigate the
// project's shared folder and drag files onto the chat surface to attach them to
// the next message (see SESSION_IMPORT_MIME consumer in the session route).

import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { listDirentsRaw, stripScopePrefix, type DirentScope } from '@/api/dirents';
import { nameOf } from '@/domain/files';
import { FileTypeIcon } from './FileTypeIcon';
import { Icon } from './Icon';
import { EmptyState } from './uiPrimitives';

/** dataTransfer MIME for dragging a shared file into the session as an attachment. */
export const SESSION_IMPORT_MIME = 'application/x-cowork-session-import';

/** Payload carried on SESSION_IMPORT_MIME — a list of files to attach. */
export type SessionImportItem = { globalPath: string; filename: string };

interface SharedFilesBrowserProps {
  projectId: string;
  /** Project name, shown as the breadcrumb root next to the home icon. */
  projectName?: string;
  /** Import the given shared files into the session (inline button fallback). */
  onImport: (items: SessionImportItem[]) => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

type Row =
  | { kind: 'dir'; name: string; relPath: string }
  | { kind: 'file'; name: string; relPath: string; globalPath: string; bytes?: number | null };

export function SharedFilesBrowser({ projectId, projectName, onImport }: SharedFilesBrowserProps) {
  const { t } = useTranslation('session');
  const scope: DirentScope = { kind: 'shared', projectId };

  // One recursive fetch; folder navigation is done in-memory by relative depth.
  const { data: rawEntries = [], isLoading } = useQuery({
    queryKey: ['dirents', 'shared', projectId],
    queryFn: () => listDirentsRaw(scope, true),
    enabled: Boolean(projectId),
  });

  // current directory, relative to the shared root ('' = root)
  const [dir, setDir] = useState('');

  // Immediate children of `dir`: entries one level below the current path.
  const rows = useMemo<Row[]>(() => {
    const prefix = dir ? `${dir}/` : '';
    const out: Row[] = [];
    for (const e of rawEntries) {
      const rel = stripScopePrefix(scope, e.path);
      if (!rel.startsWith(prefix)) continue;
      const remainder = rel.slice(prefix.length);
      if (!remainder || remainder.includes('/')) continue; // not an immediate child
      if (e.kind === 'dir') {
        out.push({ kind: 'dir', name: remainder, relPath: rel });
      } else {
        out.push({ kind: 'file', name: nameOf(e), relPath: rel, globalPath: e.path, bytes: e.bytes });
      }
    }
    // folders first, then files; alphabetical within each group
    out.sort((a, b) =>
      a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === 'dir' ? -1 : 1,
    );
    return out;
    // scope is derived from projectId, safe to omit
  }, [rawEntries, dir, projectId]);

  const segments = dir ? dir.split('/') : [];

  function handleDragStart(e: React.DragEvent, item: SessionImportItem) {
    e.dataTransfer.effectAllowed = 'copy';
    e.dataTransfer.setData(SESSION_IMPORT_MIME, JSON.stringify([item]));
    e.dataTransfer.setData('text/plain', item.filename);

    const ghost = document.createElement('div');
    ghost.className = 'cw-drag-ghost';
    ghost.textContent = item.filename;
    document.body.appendChild(ghost);
    e.dataTransfer.setDragImage(ghost, 14, 14);
    requestAnimationFrame(() => ghost.remove());
  }

  return (
    <div className="cw-files-browser">
      {/* ── breadcrumb (the bottom switch handles returning to info) ─ */}
      <div className="cw-files-breadcrumb">
        {/* Always present (hidden at root) so navigating doesn't shift the row. */}
        <button
          type="button"
          className="cw-files-up"
          aria-label={t('shared_files.up')}
          title={t('shared_files.up')}
          disabled={!dir}
          onClick={() => setDir(segments.slice(0, -1).join('/'))}
        >
          <Icon name="arrow-left" size={14} />
        </button>
        <button type="button" className="cw-files-home" onClick={() => setDir('')} disabled={!dir}>
          <Icon name="home" size={12} />
          {projectName && <span className="cw-files-home-name">{projectName}</span>}
        </button>
        {segments.map((seg, i) => {
          const target = segments.slice(0, i + 1).join('/');
          const isLast = i === segments.length - 1;
          return (
            <span key={target} className="cw-files-crumb">
              <Icon name="chevron-right" size={11} />
              <button type="button" onClick={() => setDir(target)} disabled={isLast}>{seg}</button>
            </span>
          );
        })}
      </div>

      {/* ── rows ─────────────────────────────────────────────────── */}
      <div className="cw-files-browser-list">
        {rows.map((row) =>
          row.kind === 'dir' ? (
            <button
              type="button"
              className="cw-artifact-row cw-files-dir-row"
              key={row.relPath}
              onClick={() => setDir(row.relPath)}
            >
              <span className="cw-artifact-name">
                <Icon name="folder" size={16} />
                {row.name}
              </span>
              <Icon name="chevron-right" size={13} />
            </button>
          ) : (
            <div
              className="cw-artifact-row"
              key={row.relPath}
              draggable
              onDragStart={(e) => handleDragStart(e, { globalPath: row.globalPath, filename: row.name })}
              style={{ cursor: 'grab' }}
              title={t('shared_files.drag_hint')}
            >
              <span className="cw-artifact-name">
                <FileTypeIcon filename={row.name} size={16} />
                {row.name}
              </span>
              <span className="cw-artifact-size">{row.bytes != null ? formatBytes(row.bytes) : ''}</span>
              <div className="cw-artifact-menu-wrap">
                <button
                  type="button"
                  aria-label={t('shared_files.import')}
                  title={t('shared_files.import')}
                  onClick={() => onImport([{ globalPath: row.globalPath, filename: row.name }])}
                >
                  <Icon name="plus" size={13} />
                </button>
              </div>
            </div>
          ),
        )}

        {!isLoading && rows.length === 0 && (
          <EmptyState chip="📁" title={t('shared_files.empty_title')} body={t('shared_files.empty_body')} />
        )}
      </div>
    </div>
  );
}
