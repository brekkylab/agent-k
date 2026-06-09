// SharedFilesBrowser — a Files-page-style browser that takes over the session
// sidebar. Opened from the sidebar's "browse" button; lets the user navigate the
// project's shared folder and drag files onto the chat surface to attach them to
// the next message (see SESSION_IMPORT_MIME consumer in the session route).

import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
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

  // ── multi-select (rubber-band marquee + click) ───────────────────
  // Keyed global path → filename, so a multi-file drag has names even if the
  // selection spans a folder we've since navigated out of.
  const [selected, setSelected] = useState<Map<string, string>>(new Map());
  const [dragRect, setDragRect] = useState<{ left: number; top: number; width: number; height: number } | null>(null);
  const dragOriginRef = useRef<{ x: number; y: number; base: Map<string, string>; additive: boolean } | null>(null);
  const didDragRef = useRef(false);
  const listRef = useRef<HTMLDivElement>(null);
  const lastPointerRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const lastScrollTopRef = useRef(0);
  const DRAG_THRESHOLD = 4;

  // Selection is per-view — reset when changing folders.
  useEffect(() => { setSelected(new Map()); }, [dir]);

  function onListMouseDown(e: React.MouseEvent) {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest('.cw-sf-add')) return; // let the + button work
    const rowEl = (e.target as HTMLElement).closest('[data-sf-path]');
    const additive = e.shiftKey || e.metaKey || e.ctrlKey;
    dragOriginRef.current = {
      x: e.clientX,
      y: e.clientY,
      base: rowEl || additive ? new Map(selected) : new Map(),
      additive,
    };
    lastPointerRef.current = { x: e.clientX, y: e.clientY };
    lastScrollTopRef.current = listRef.current?.scrollTop ?? 0;
    didDragRef.current = false;
    if (!rowEl && !additive) setSelected(new Map());
  }

  // Global marquee tracking — draw a rectangle and select intersecting file rows.
  useEffect(() => {
    // Compute the rect (viewport coords) from the anchor to the pointer, draw it,
    // and select every file row it intersects. Rows use getBoundingClientRect, so
    // scrolled-off rows still test correctly once the anchor is scroll-adjusted.
    function applyMarquee(clientX: number, clientY: number) {
      const origin = dragOriginRef.current;
      if (!origin) return;
      const dx = clientX - origin.x;
      const dy = clientY - origin.y;
      if (Math.abs(dx) < DRAG_THRESHOLD && Math.abs(dy) < DRAG_THRESHOLD && !dragRect) return;
      didDragRef.current = true;
      const left = Math.min(origin.x, clientX);
      const top = Math.min(origin.y, clientY);
      const width = Math.abs(dx);
      const height = Math.abs(dy);
      setDragRect({ left, top, width, height });
      const r = { left, top, right: left + width, bottom: top + height };
      const next = new Map(origin.base);
      for (const el of document.querySelectorAll<HTMLElement>('[data-sf-path]')) {
        const rect = el.getBoundingClientRect();
        if (rect.left < r.right && rect.right > r.left && rect.top < r.bottom && rect.bottom > r.top) {
          const gp = el.dataset.sfPath;
          if (gp) next.set(gp, el.dataset.sfName ?? gp);
        }
      }
      setSelected(next);
    }
    function onMove(e: MouseEvent) {
      lastPointerRef.current = { x: e.clientX, y: e.clientY };
      applyMarquee(e.clientX, e.clientY);
    }
    // Scrolling the list while dragging: the anchor is pinned to content, so
    // shift it by the scroll delta and re-test (mousemove may not fire on a wheel).
    function onScroll() {
      const origin = dragOriginRef.current;
      const list = listRef.current;
      if (!origin || !list) return;
      const delta = list.scrollTop - lastScrollTopRef.current;
      lastScrollTopRef.current = list.scrollTop;
      origin.y -= delta;
      const { x, y } = lastPointerRef.current;
      applyMarquee(x, y);
    }
    function onUp() {
      const dragged = didDragRef.current && dragOriginRef.current;
      dragOriginRef.current = null;
      setDragRect(null);
      if (dragged) {
        // Swallow the click that follows a marquee so it doesn't reset selection.
        const swallow = (ev: Event) => { ev.stopPropagation(); ev.preventDefault(); window.removeEventListener('click', swallow, true); };
        window.addEventListener('click', swallow, true);
      }
    }
    const list = listRef.current;
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    list?.addEventListener('scroll', onScroll);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      list?.removeEventListener('scroll', onScroll);
    };
  }, [dragRect]);

  function selectClick(e: React.MouseEvent, globalPath: string, filename: string) {
    const additive = e.shiftKey || e.metaKey || e.ctrlKey;
    setSelected((prev) => {
      if (!additive) return new Map([[globalPath, filename]]);
      const next = new Map(prev);
      if (next.has(globalPath)) next.delete(globalPath);
      else next.set(globalPath, filename);
      return next;
    });
  }

  function handleDragStart(e: React.DragEvent, item: SessionImportItem) {
    // A native drag is starting — cancel any pending marquee origin.
    dragOriginRef.current = null;
    setDragRect(null);
    // Custom MIME only — omit text/plain so external apps can't accept the drop.
    e.dataTransfer.effectAllowed = 'copy';
    // Dragging a selected row carries the whole selection; otherwise just this file.
    const items: SessionImportItem[] = selected.has(item.globalPath) && selected.size > 1
      ? [...selected].map(([globalPath, filename]) => ({ globalPath, filename }))
      : [item];
    e.dataTransfer.setData(SESSION_IMPORT_MIME, JSON.stringify(items));

    const ghost = document.createElement('div');
    ghost.className = 'cw-drag-ghost';
    ghost.textContent = items.length === 1 ? item.filename : t('shared_files.drag_items', { count: items.length });
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
      <div className="cw-files-browser-list" ref={listRef} onMouseDown={onListMouseDown}>
        {rows.map((row) =>
          row.kind === 'dir' ? (
            <button
              type="button"
              className="cw-sf-row cw-sf-dir"
              key={row.relPath}
              onClick={() => setDir(row.relPath)}
            >
              <span className="cw-sf-grip-spacer" aria-hidden="true" />
              <Icon name="folder" size={18} />
              <span className="cw-file-label">{row.name}</span>
              <Icon name="chevron-right" size={14} />
            </button>
          ) : (
            <div
              className={`cw-sf-row${selected.has(row.globalPath) ? ' is-selected' : ''}`}
              key={row.relPath}
              data-sf-path={row.globalPath}
              data-sf-name={row.name}
              draggable
              onDragStart={(e) => handleDragStart(e, { globalPath: row.globalPath, filename: row.name })}
              onClick={(e) => selectClick(e, row.globalPath, row.name)}
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

      {/* Portal to body: an ancestor's transform (the sliding sidebar track)
          would otherwise re-anchor this fixed overlay and overflow:hidden clip it. */}
      {dragRect && createPortal(
        <div
          className="cw-marquee"
          style={{ left: dragRect.left, top: dragRect.top, width: dragRect.width, height: dragRect.height }}
        />,
        document.body,
      )}
    </div>
  );
}
