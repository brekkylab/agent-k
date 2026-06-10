// SharedFilesBrowser — a Files-page-style browser that takes over the session
// sidebar. Opened from the sidebar's "browse" button; lets the user navigate the
// project's shared folder and drag files onto the chat surface to attach them to
// the next message (see SESSION_IMPORT_MIME consumer in the session route).

import { useEffect, useMemo, useRef, useState } from 'react';
import { MarqueeOverlay } from '@/components/MarqueeOverlay';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { listDirentsRaw, stripScopePrefix, type DirentScope } from '@/api/dirents';
import { expandDirentPaths, isHiddenName, nameOf } from '@/domain/files';
import { FilePreviewModal } from '@/components/FilePreviewModal';
import { useMarqueeSelection } from '@/lib/useMarqueeSelection';
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
  | { kind: 'dir'; name: string; relPath: string; globalPath: string }
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
  // Double-clicked file shown in the in-app preview modal (global path).
  const [previewPath, setPreviewPath] = useState<string | null>(null);

  // Immediate children of `dir`: entries one level below the current path.
  const rows = useMemo<Row[]>(() => {
    const prefix = dir ? `${dir}/` : '';
    const out: Row[] = [];
    for (const e of rawEntries) {
      const rel = stripScopePrefix(scope, e.path);
      if (!rel.startsWith(prefix)) continue;
      const remainder = rel.slice(prefix.length);
      if (!remainder || remainder.includes('/')) continue; // not an immediate child
      if (isHiddenName(remainder)) continue; // hide dotfiles (.keep placeholders, etc.)
      if (e.kind === 'dir') {
        out.push({ kind: 'dir', name: remainder, relPath: rel, globalPath: e.path });
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

  // ── multi-select (rubber-band marquee + click) ───────────────────
  // Selected file global paths, per-view (cleared on folder change). Filenames
  // for the drag payload are derived from each path's basename.
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const listRef = useRef<HTMLDivElement>(null);

  // Selection is per-view — reset when changing folders.
  useEffect(() => { setSelected(new Set()); }, [dir]);

  const marquee = useMarqueeSelection({
    scrollRef: listRef,
    itemSelector: '[data-sf-path]',
    keyAttr: 'sfPath',
    ignoreSelector: '.cw-sf-add', // let the inline + button work
    getSelection: () => selected,
    setSelection: setSelected,
  });

  function selectClick(e: React.MouseEvent, globalPath: string) {
    const additive = e.shiftKey || e.metaKey || e.ctrlKey;
    setSelected((prev) => {
      if (!additive) return new Set([globalPath]);
      const next = new Set(prev);
      if (next.has(globalPath)) next.delete(globalPath);
      else next.add(globalPath);
      return next;
    });
  }

  // One drag handler for both file and folder rows. Dragging a selected row
  // carries the whole selection (folders expanded to their files); otherwise
  // just the dragged row.
  function handleRowDragStart(e: React.DragEvent, globalPath: string) {
    marquee.cancel(); // a native drag is starting — abort any pending marquee
    const sources = selected.has(globalPath) && selected.size > 1 ? [...selected] : [globalPath];
    const items = expandDirentPaths(rawEntries, sources);
    // An empty folder still drags (drop is just a no-op) — don't block the gesture.
    // Custom MIME only — omit text/plain so external apps can't accept the drop.
    e.dataTransfer.effectAllowed = 'copy';
    e.dataTransfer.setData(SESSION_IMPORT_MIME, JSON.stringify(items));

    const ghost = document.createElement('div');
    ghost.className = 'cw-drag-ghost';
    ghost.textContent = items.length === 1
      ? items[0]!.filename
      : items.length === 0
        ? (globalPath.split('/').pop() ?? globalPath)
        : t('shared_files.drag_items', { count: items.length });
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
      <div className="cw-files-browser-list" ref={listRef} onMouseDown={marquee.onMouseDown}>
        {rows.map((row) =>
          row.kind === 'dir' ? (
            <div
              className={`cw-sf-row cw-sf-dir${selected.has(row.globalPath) ? ' is-selected' : ''}`}
              key={row.relPath}
              data-sf-path={row.globalPath}
              data-sf-name={row.name}
              draggable
              onDragStart={(e) => handleRowDragStart(e, row.globalPath)}
              onClick={() => setDir(row.relPath)}
            >
              <span className="cw-sf-grip" aria-hidden="true">
                <svg width="10" height="14" viewBox="0 0 10 14" fill="currentColor">
                  <circle cx="2" cy="2" r="1.2" /><circle cx="8" cy="2" r="1.2" />
                  <circle cx="2" cy="7" r="1.2" /><circle cx="8" cy="7" r="1.2" />
                  <circle cx="2" cy="12" r="1.2" /><circle cx="8" cy="12" r="1.2" />
                </svg>
              </span>
              <Icon name="folder" size={18} />
              <span className="cw-file-label">{row.name}</span>
              <Icon name="chevron-right" size={14} />
            </div>
          ) : (
            <div
              className={`cw-sf-row${selected.has(row.globalPath) ? ' is-selected' : ''}`}
              key={row.relPath}
              data-sf-path={row.globalPath}
              data-sf-name={row.name}
              draggable
              onDragStart={(e) => handleRowDragStart(e, row.globalPath)}
              onClick={(e) => selectClick(e, row.globalPath)}
              onDoubleClick={() => setPreviewPath(row.globalPath)}
              title={`${row.name}${row.bytes != null ? ` · ${formatBytes(row.bytes)}` : ''}`}
            >
              <span className="cw-sf-grip" aria-hidden="true">
                <svg width="10" height="14" viewBox="0 0 10 14" fill="currentColor">
                  <circle cx="2" cy="2" r="1.2" /><circle cx="8" cy="2" r="1.2" />
                  <circle cx="2" cy="7" r="1.2" /><circle cx="8" cy="7" r="1.2" />
                  <circle cx="2" cy="12" r="1.2" /><circle cx="8" cy="12" r="1.2" />
                </svg>
              </span>
              <FileTypeIcon filename={row.name} size={18} />
              <span className="cw-file-label">{row.name}</span>
              <button
                type="button"
                className="cw-sf-add"
                aria-label={t('shared_files.import')}
                title={t('shared_files.import')}
                onClick={(e) => { e.stopPropagation(); onImport([{ globalPath: row.globalPath, filename: row.name }]); }}
              >
                <Icon name="plus" size={13} />
              </button>
            </div>
          ),
        )}

        {!isLoading && rows.length === 0 && (
          <EmptyState chip="📁" title={t('shared_files.empty_title')} body={t('shared_files.empty_body')} />
        )}
      </div>

      <MarqueeOverlay rect={marquee.dragRect} />

      {previewPath && (
        <FilePreviewModal globalPath={previewPath} onClose={() => setPreviewPath(null)} />
      )}
    </div>
  );
}
