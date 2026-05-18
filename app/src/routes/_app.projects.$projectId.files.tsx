// Files — sidebar tree + main browser, list/grid views, multi-select.
// Modifier conventions follow Google Drive / OneDrive:
//   click           — select single, anchor = clicked
//   shift+click     — range from anchor to clicked
//   meta/ctrl+click — toggle clicked, anchor = clicked
//   dblclick        — open (folder → currentPath / file → download)
//   esc / backdrop  — clear selection
//   delete/backspace— bulk delete (with ConfirmDialog)
//   meta/ctrl+a     — select all rows in current view
//   ↑/↓             — move focus + select prev/next row
//   drag empty area — rubber-band rectangle select

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { createFileRoute } from '@tanstack/react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  createFolder,
  deleteDirent,
  downloadFile,
  listDirentsRaw,
  uploadFiles,
  type UploadResult,
} from '@/api/dirents';
import { getProject } from '@/api/projects';
import { Icon } from '@/components/Icon';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { NewFolderDialog } from '@/components/NewFolderDialog';
import { EmptyState, IconPocket } from '@/components/uiPrimitives';
import { useToastStore } from '@/components/Toast';
import { ApiError } from '@/api/client';
import {
  ancestorPaths,
  buildFolderTree,
  countDescendants,
  fileTypeClass,
  fileTypeIcon,
  listDirectChildren,
  nameOf,
  type FolderNode,
} from '@/domain/files';
import type { BackendDirent } from '@/api/backend-types';

type ViewMode = 'list' | 'grid';
const VIEW_KEY = 'cowork.files.viewMode';
const DRAG_THRESHOLD = 5; // px — under this we treat mousedown as click

export const Route = createFileRoute('/_app/projects/$projectId/files')({
  component: FilesPage,
});

function FilesPage() {
  const { projectId } = Route.useParams();
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const project = useQuery({ queryKey: ['project', projectId], queryFn: () => getProject(projectId) });
  const dirents = useQuery({
    queryKey: ['dirents', projectId],
    queryFn: () => listDirentsRaw(projectId),
  });

  const entries = dirents.data ?? [];

  // ── Navigation state ─────────────────────────────────────────────
  const [currentPath, setCurrentPath] = useState<string[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  useEffect(() => {
    setExpanded((prev) => {
      const next = new Set(prev);
      ancestorPaths(currentPath).forEach((p) => next.add(p));
      return next;
    });
  }, [currentPath]);

  // ── Selection state ──────────────────────────────────────────────
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const anchorRef = useRef<string | null>(null);

  useEffect(() => {
    setSelectedPaths(new Set());
    anchorRef.current = null;
  }, [currentPath]);

  // ── View mode ────────────────────────────────────────────────────
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    if (typeof window === 'undefined') return 'list';
    return window.localStorage.getItem(VIEW_KEY) === 'grid' ? 'grid' : 'list';
  });
  useEffect(() => {
    if (typeof window !== 'undefined') window.localStorage.setItem(VIEW_KEY, viewMode);
  }, [viewMode]);

  // ── Search ───────────────────────────────────────────────────────
  const [query, setQuery] = useState('');
  const deferred = query.trim().toLowerCase();

  // ── Derived data ─────────────────────────────────────────────────
  const tree = useMemo(() => buildFolderTree(entries), [entries]);
  const view = useMemo(() => {
    if (deferred) {
      const matches = entries.filter((e) =>
        e.kind === 'file' && !nameOf(e).startsWith('.') && nameOf(e).toLowerCase().includes(deferred),
      );
      return { folders: [] as BackendDirent[], files: matches, isSearch: true };
    }
    return { ...listDirectChildren(entries, currentPath), isSearch: false };
  }, [entries, currentPath, deferred]);

  const allRows = useMemo<BackendDirent[]>(() => [...view.folders, ...view.files], [view]);
  const rowIndex = useMemo(() => {
    const m = new Map<string, number>();
    allRows.forEach((r, i) => m.set(r.path, i));
    return m;
  }, [allRows]);

  const breadcrumbs = useMemo(() => {
    const crumbs = [{ label: project.data?.name ?? 'Project', segments: [] as string[] }];
    currentPath.forEach((seg, i) => crumbs.push({ label: seg, segments: currentPath.slice(0, i + 1) }));
    return crumbs;
  }, [project.data?.name, currentPath]);

  const selectedEntries = useMemo<BackendDirent[]>(
    () => Array.from(selectedPaths).map((p) => entries.find((e) => e.path === p)).filter((e): e is BackendDirent => Boolean(e)),
    [selectedPaths, entries],
  );

  // ── Mutations ────────────────────────────────────────────────────
  const targetPathFor = (file: File) =>
    currentPath.length > 0 ? `${currentPath.join('/')}/${file.name}` : file.name;

  const uploadMutation = useMutation({
    mutationFn: async (files: File[]): Promise<UploadResult> => {
      const items = files.map((file) => ({ file, targetPath: targetPathFor(file) }));
      return uploadFiles(projectId, items);
    },
    onSuccess: async (result) => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', projectId] });
      const ok = result.succeeded.length;
      const ko = result.failed.length;
      if (ko === 0) {
        showToast(ok === 1 ? '파일이 업로드되었습니다' : `${ok}개 파일이 업로드되었습니다`);
      } else {
        showToast(
          `${ok}개 업로드, ${ko}개 실패`,
          result.failed.map((f) => `${f.path || '(이름 없음)'} — ${f.error}`),
        );
      }
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'upload failed';
      showToast(`업로드 실패: ${msg}`);
    },
  });

  const [folderDialogOpen, setFolderDialogOpen] = useState(false);
  const existingFolderNames = useMemo(
    () => view.folders.map((f) => nameOf(f)),
    [view.folders],
  );

  const folderMutation = useMutation({
    mutationFn: (name: string) => {
      const cleaned = name.trim().replace(/^\/+|\/+$/g, '');
      const fullPath = currentPath.length > 0 ? `${currentPath.join('/')}/${cleaned}` : cleaned;
      return createFolder(projectId, fullPath);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', projectId] });
      showToast('폴더가 생성되었습니다');
      setFolderDialogOpen(false);
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'mkdir failed';
      showToast(`폴더 생성 실패: ${msg}`);
    },
  });

  const downloadMutation = useMutation({
    mutationFn: (entry: BackendDirent) => downloadFile(projectId, entry.path),
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'download failed';
      showToast(`다운로드 실패: ${msg}`);
    },
  });

  const [pendingDelete, setPendingDelete] = useState<BackendDirent[] | null>(null);

  const bulkDeleteMutation = useMutation({
    mutationFn: async (targets: BackendDirent[]) => {
      const results = await Promise.allSettled(targets.map((t) => deleteDirent(projectId, t.path)));
      return { targets, results };
    },
    onSuccess: async ({ targets, results }) => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', projectId] });
      const fails: Array<{ path: string; error: string }> = [];
      let okCount = 0;
      results.forEach((r, idx) => {
        if (r.status === 'rejected') {
          const reason = r.reason instanceof ApiError ? r.reason.message
            : r.reason instanceof Error ? r.reason.message
            : String(r.reason);
          fails.push({ path: targets[idx]!.path, error: reason });
        } else {
          okCount += 1;
        }
      });
      if (fails.length === 0) {
        showToast(okCount === 1 ? '삭제되었습니다' : `${okCount}개 항목이 삭제되었습니다`);
      } else {
        showToast(`${okCount}개 삭제, ${fails.length}개 실패`, fails.map((f) => `${f.path} — ${f.error}`));
      }
      setSelectedPaths(new Set());
      anchorRef.current = null;
      setPendingDelete(null);
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'delete failed';
      showToast(`삭제 실패: ${msg}`);
    },
  });

  // ── Drag & drop (upload) ─────────────────────────────────────────
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [isDraggingOver, setIsDraggingOver] = useState(false);
  const dragDepthRef = useRef(0);

  function onDropFiles(fileList: FileList | null) {
    if (!fileList || fileList.length === 0) return;
    uploadMutation.mutate(Array.from(fileList));
  }
  function openUpload() { fileInputRef.current?.click(); }
  function openFolderDialog() { setFolderDialogOpen(true); }

  // ── Selection handlers ───────────────────────────────────────────
  const handleSelect = useCallback((entry: BackendDirent, ev: React.MouseEvent | React.KeyboardEvent) => {
    const path = entry.path;
    const idx = rowIndex.get(path);
    const shift = 'shiftKey' in ev && ev.shiftKey;
    const meta = ('metaKey' in ev && ev.metaKey) || ('ctrlKey' in ev && ev.ctrlKey);

    if (shift && anchorRef.current != null) {
      const aIdx = rowIndex.get(anchorRef.current);
      if (aIdx != null && idx != null) {
        const [lo, hi] = aIdx <= idx ? [aIdx, idx] : [idx, aIdx];
        setSelectedPaths(new Set(allRows.slice(lo, hi + 1).map((r) => r.path)));
        return;
      }
    }
    if (meta) {
      setSelectedPaths((prev) => {
        const next = new Set(prev);
        if (next.has(path)) next.delete(path);
        else next.add(path);
        return next;
      });
      anchorRef.current = path;
      return;
    }
    setSelectedPaths(new Set([path]));
    anchorRef.current = path;
  }, [allRows, rowIndex]);

  const openEntry = useCallback((entry: BackendDirent) => {
    if (entry.kind === 'dir') setCurrentPath(entry.path.split('/').filter(Boolean));
    else downloadMutation.mutate(entry);
  }, [downloadMutation]);

  const clearSelection = useCallback(() => {
    setSelectedPaths(new Set());
    anchorRef.current = null;
  }, []);

  const requestBulkDelete = useCallback(() => {
    if (selectedEntries.length > 0) setPendingDelete(selectedEntries);
  }, [selectedEntries]);

  const bulkDownload = useCallback(() => {
    const files = selectedEntries.filter((e) => e.kind === 'file');
    if (files.length === 0) { showToast('다운로드할 파일이 선택되지 않았습니다'); return; }
    files.forEach((f) => downloadMutation.mutate(f));
  }, [selectedEntries, downloadMutation, showToast]);

  // ── Outside click clears selection ───────────────────────────────
  // If the user clicks anywhere that isn't a file-pane interaction, the
  // floating bulk toolbar, or an open dialog, treat it as "dismiss".
  useEffect(() => {
    if (selectedPaths.size === 0) return;
    function onDocMouseDown(e: MouseEvent) {
      const t = e.target as HTMLElement | null;
      if (!t) return;
      if (t.closest('.cw-file-pane')) return;
      if (t.closest('.cw-dialog')) return;
      clearSelection();
    }
    document.addEventListener('mousedown', onDocMouseDown);
    return () => document.removeEventListener('mousedown', onDocMouseDown);
  }, [selectedPaths.size, clearSelection]);

  // ── Keyboard shortcuts (window-level) ────────────────────────────
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const inField = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);

      if (e.key === 'Escape') { if (!inField) clearSelection(); return; }
      if (inField) return;

      if ((e.key === 'Delete' || e.key === 'Backspace') && selectedPaths.size > 0) {
        e.preventDefault();
        requestBulkDelete();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a') {
        if (allRows.length === 0) return;
        e.preventDefault();
        setSelectedPaths(new Set(allRows.map((r) => r.path)));
        anchorRef.current = allRows[allRows.length - 1]?.path ?? null;
        return;
      }

      // Arrow navigation: only when focus is inside a file row/card and there are rows.
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        const rowEl = target?.closest('[data-row-index]') as HTMLElement | null;
        if (!rowEl || allRows.length === 0) return;
        e.preventDefault();
        const curIdx = Number(rowEl.dataset.rowIndex);
        const step = e.key === 'ArrowDown' ? 1 : -1;
        const next = Math.max(0, Math.min(allRows.length - 1, curIdx + step));
        const nextEntry = allRows[next];
        if (!nextEntry) return;
        if (e.shiftKey && anchorRef.current != null) {
          const aIdx = rowIndex.get(anchorRef.current);
          if (aIdx != null) {
            const [lo, hi] = aIdx <= next ? [aIdx, next] : [next, aIdx];
            setSelectedPaths(new Set(allRows.slice(lo, hi + 1).map((r) => r.path)));
          }
        } else {
          setSelectedPaths(new Set([nextEntry.path]));
          anchorRef.current = nextEntry.path;
        }
        const nextEl = document.querySelector<HTMLElement>(`[data-row-index="${next}"]`);
        nextEl?.focus();
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [allRows, rowIndex, selectedPaths.size, clearSelection, requestBulkDelete]);

  function toggleExpand(path: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  // ── Rubber-band selection ────────────────────────────────────────
  // Allows marquee to start anywhere — empty space OR on top of a row.
  // A row's click handler only fires when the mouse hasn't moved beyond
  // DRAG_THRESHOLD between mousedown and mouseup. If it did move, we
  // swallow the synthesized click in capture phase to keep the marquee
  // selection from being reset by the underlying row.
  const bodyRef = useRef<HTMLDivElement>(null);
  const dragOriginRef = useRef<{
    x: number;
    y: number;
    basePaths: Set<string>;
    startedOnRow: boolean;
    additive: boolean;
  } | null>(null);
  const didDragRef = useRef(false);
  const [dragRect, setDragRect] = useState<{ left: number; top: number; width: number; height: number } | null>(null);

  const onBodyMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.button !== 0) return;
    const rowEl = (e.target as HTMLElement).closest('[data-row-index]');
    const additive = e.shiftKey || e.metaKey || e.ctrlKey;
    dragOriginRef.current = {
      x: e.clientX,
      y: e.clientY,
      // When starting on a row we keep current selection as a base — the row's
      // own click handler will resolve the single-select case if no drag.
      // When starting on empty space without modifier we clear immediately so
      // the user sees the deselect feedback even before they drag.
      basePaths: rowEl || additive ? new Set(selectedPaths) : new Set(),
      startedOnRow: !!rowEl,
      additive,
    };
    didDragRef.current = false;
    if (!rowEl && !additive) clearSelection();
  }, [selectedPaths, clearSelection]);

  useEffect(() => {
    function onMove(e: MouseEvent) {
      const origin = dragOriginRef.current;
      if (!origin) return;
      const dx = e.clientX - origin.x;
      const dy = e.clientY - origin.y;
      if (Math.abs(dx) < DRAG_THRESHOLD && Math.abs(dy) < DRAG_THRESHOLD && !dragRect) return;
      didDragRef.current = true;

      const left = Math.min(origin.x, e.clientX);
      const top = Math.min(origin.y, e.clientY);
      const width = Math.abs(dx);
      const height = Math.abs(dy);
      setDragRect({ left, top, width, height });

      const rows = Array.from(document.querySelectorAll<HTMLElement>('[data-row-index]'));
      const next = new Set(origin.basePaths);
      const r = { left, top, right: left + width, bottom: top + height };
      for (const row of rows) {
        const rect = row.getBoundingClientRect();
        const inside = rect.left < r.right && rect.right > r.left && rect.top < r.bottom && rect.bottom > r.top;
        if (inside) {
          const path = row.dataset.rowPath;
          if (path) next.add(path);
        }
      }
      setSelectedPaths(next);
    }
    function onUp() {
      const origin = dragOriginRef.current;
      dragOriginRef.current = null;
      setDragRect(null);
      if (!origin) return;
      if (didDragRef.current) {
        // Capture-phase listener fires *before* any onClick — swallow the
        // synthetic click that follows this mouseup so the underlying row's
        // onClick handler doesn't reset our marquee selection.
        const swallow = (ev: Event) => {
          ev.stopPropagation();
          ev.preventDefault();
          window.removeEventListener('click', swallow, true);
        };
        window.addEventListener('click', swallow, true);
      }
    }
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
  }, [dragRect]);

  // ── Confirm dialog copy ──────────────────────────────────────────
  const deleteCopy = useMemo(() => describeBulkDelete(pendingDelete, entries), [pendingDelete, entries]);
  const currentPathKey = currentPath.join('/');

  return (
    <section className="cw-page cw-files-page cw-page-enter">
      <div className="cw-page-head">
        <div>
          <h1>Files</h1>
          <p>Shared with the whole project. Select files, pin them to a session, then generate artifacts from those citations.</p>
        </div>
        <div>
          <button className="cw-btn-secondary" onClick={openFolderDialog} disabled={folderMutation.isPending}>
            <IconPocket tone="add" icon="plus" compact /> New folder
          </button>
          <button className="cw-btn-primary" onClick={openUpload} disabled={uploadMutation.isPending}>
            <IconPocket tone="add" icon="upload" compact /> {uploadMutation.isPending ? 'Uploading…' : 'Upload'}
          </button>
          <input
            ref={fileInputRef}
            type="file"
            hidden
            multiple
            onChange={(e) => { onDropFiles(e.target.files); e.target.value = ''; }}
          />
        </div>
      </div>

      <div className="cw-file-browser">
        <aside className="cw-sidebar-tree">
          <button
            type="button"
            className={`cw-tree-row cw-tree-root${currentPath.length === 0 ? ' is-active' : ''}`}
            onClick={() => setCurrentPath([])}
          >
            <Icon name="folder" size={14} />
            <span>{project.data?.name ?? 'Project'}</span>
          </button>
          {tree.map((node) => (
            <TreeBranch
              key={node.path}
              node={node}
              depth={0}
              currentPathKey={currentPathKey}
              expanded={expanded}
              entries={entries}
              onSelect={(segments) => setCurrentPath(segments)}
              onToggle={toggleExpand}
            />
          ))}
        </aside>

        <section className="cw-file-pane">
          <header className="cw-file-pane-head">
            {selectedPaths.size >= 1 ? (
              <div className="cw-bulk-toolbar cw-bulk-toolbar-inline" role="toolbar" aria-label="bulk actions">
                <span className="cw-bulk-count">{selectedPaths.size}개 선택됨</span>
                <button type="button" className="cw-btn-secondary" onClick={bulkDownload}>
                  <Icon name="download" size={13} /> Download
                </button>
                <button type="button" className="cw-btn-secondary cw-btn-destructive" onClick={requestBulkDelete}>
                  <Icon name="trash" size={13} /> Delete
                </button>
                <button type="button" className="cw-bulk-clear" onClick={clearSelection} aria-label="선택 해제">
                  <Icon name="x" size={13} />
                </button>
              </div>
            ) : (
              <nav className="cw-breadcrumb" aria-label="folder path">
                {breadcrumbs.map((crumb, i) => {
                  const isLast = i === breadcrumbs.length - 1;
                  return (
                    <span key={`${crumb.segments.join('/')}-${i}`}>
                      {i > 0 && <span className="cw-breadcrumb-sep"><Icon name="chevron-right" size={12} /></span>}
                      {isLast ? <b>{crumb.label}</b> : (
                        <button type="button" onClick={() => setCurrentPath(crumb.segments)}>{crumb.label}</button>
                      )}
                    </span>
                  );
                })}
              </nav>
            )}
            <div className="cw-file-pane-tools">
              <label className="cw-files-search">
                <Icon name="search" size={12} />
                <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="이 프로젝트에서 검색" />
              </label>
              <div className="cw-view-toggle" role="tablist" aria-label="보기 방식">
                <button type="button" role="tab" aria-selected={viewMode === 'list'} className={viewMode === 'list' ? 'is-active' : ''} onClick={() => setViewMode('list')} aria-label="리스트 보기"><Icon name="list" size={14} /></button>
                <button type="button" role="tab" aria-selected={viewMode === 'grid'} className={viewMode === 'grid' ? 'is-active' : ''} onClick={() => setViewMode('grid')} aria-label="그리드 보기"><Icon name="grid" size={14} /></button>
            </div>
            </div>
          </header>

          <div
            ref={bodyRef}
            className={`cw-file-body cw-view-${viewMode}${isDraggingOver ? ' is-over' : ''}`}
            onClick={(e) => { if (e.target === e.currentTarget) clearSelection(); }}
            onMouseDown={onBodyMouseDown}
            onDragEnter={(e) => { e.preventDefault(); dragDepthRef.current += 1; setIsDraggingOver(true); }}
            onDragOver={(e) => { e.preventDefault(); }}
            onDragLeave={(e) => {
              e.preventDefault();
              dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
              if (dragDepthRef.current === 0) setIsDraggingOver(false);
            }}
            onDrop={(e) => {
              e.preventDefault();
              dragDepthRef.current = 0;
              setIsDraggingOver(false);
              onDropFiles(e.dataTransfer.files);
            }}
          >
            {allRows.length === 0 ? (
              <EmptyState
                title={view.isSearch ? '검색 결과 없음' : '이 폴더는 비어 있습니다'}
                body={view.isSearch
                  ? '다른 키워드로 시도하거나 검색을 지우세요.'
                  : '파일을 드래그하거나 Upload를 눌러 첫 파일을 올려보세요.'}
                action={view.isSearch ? undefined : 'Upload file'}
                onAction={view.isSearch ? undefined : openUpload}
                chip={<Icon name="folder" size={16} />}
              />
            ) : viewMode === 'list' ? (
              <div className="cw-file-table">
                {allRows.map((entry, idx) => (
                  <ListRow
                    key={entry.path}
                    entry={entry}
                    index={idx}
                    entries={entries}
                    selected={selectedPaths.has(entry.path)}
                    showPath={view.isSearch}
                    onSelect={handleSelect}
                    onOpen={openEntry}
                    onDelete={(e) => setPendingDelete([e])}
                  />
                ))}
              </div>
            ) : (
              <div className="cw-grid-cards">
                {allRows.map((entry, idx) => (
                  <GridCard
                    key={entry.path}
                    entry={entry}
                    index={idx}
                    entries={entries}
                    selected={selectedPaths.has(entry.path)}
                    showPath={view.isSearch}
                    onSelect={handleSelect}
                    onOpen={openEntry}
                    onDelete={(e) => setPendingDelete([e])}
                  />
                ))}
              </div>
            )}
          </div>
        </section>
      </div>

      {dragRect && createPortal(
        <div
          className="cw-marquee"
          style={{ left: dragRect.left, top: dragRect.top, width: dragRect.width, height: dragRect.height }}
        />,
        document.body,
      )}

      {folderDialogOpen && (
        <NewFolderDialog
          existingNames={existingFolderNames}
          pending={folderMutation.isPending}
          onConfirm={(name) => folderMutation.mutate(name)}
          onClose={() => { if (!folderMutation.isPending) setFolderDialogOpen(false); }}
        />
      )}

      {pendingDelete && deleteCopy && (
        <ConfirmDialog
          title={deleteCopy.title}
          body={deleteCopy.body}
          confirmLabel="삭제"
          destructive
          pending={bulkDeleteMutation.isPending}
          onConfirm={() => bulkDeleteMutation.mutate(pendingDelete)}
          onClose={() => { if (!bulkDeleteMutation.isPending) setPendingDelete(null); }}
        />
      )}
    </section>
  );
}

// ── Sidebar tree branch ───────────────────────────────────────────────

function TreeBranch({
  node,
  depth,
  currentPathKey,
  expanded,
  entries,
  onSelect,
  onToggle,
}: {
  node: FolderNode;
  depth: number;
  currentPathKey: string;
  expanded: Set<string>;
  entries: BackendDirent[];
  onSelect: (segments: string[]) => void;
  onToggle: (path: string) => void;
}) {
  const isActive = currentPathKey === node.path;
  const hasChildren = node.children.length > 0;
  const isOpen = expanded.has(node.path);
  const count = countDescendants(entries, node.segments);

  return (
    <>
      <div className={`cw-tree-row${isActive ? ' is-active' : ''}`} style={{ paddingLeft: 8 + depth * 14 }}>
        <button
          type="button"
          className="cw-tree-chevron"
          aria-label={isOpen ? '접기' : '펼치기'}
          onClick={(e) => { e.stopPropagation(); onToggle(node.path); }}
          style={{ visibility: hasChildren ? 'visible' : 'hidden' }}
        >
          <Icon name={isOpen ? 'chevron' : 'chevron-right'} size={12} />
        </button>
        <button type="button" className="cw-tree-label" onClick={() => onSelect(node.segments)}>
          <Icon name={isActive ? 'folder-open' : 'folder'} size={14} />
          <span>{node.name}</span>
          {count > 0 && <span className="cw-tree-count">{count}</span>}
        </button>
      </div>
      {isOpen && node.children.map((child) => (
        <TreeBranch
          key={child.path}
          node={child}
          depth={depth + 1}
          currentPathKey={currentPathKey}
          expanded={expanded}
          entries={entries}
          onSelect={onSelect}
          onToggle={onToggle}
        />
      ))}
    </>
  );
}

// ── Row / Card primitives ─────────────────────────────────────────────

interface RowProps {
  entry: BackendDirent;
  index: number;
  entries: BackendDirent[];
  selected: boolean;
  showPath: boolean;
  onSelect: (e: BackendDirent, ev: React.MouseEvent | React.KeyboardEvent) => void;
  onOpen: (e: BackendDirent) => void;
  onDelete: (e: BackendDirent) => void;
}

function iconClass(entry: BackendDirent): string {
  return entry.kind === 'dir' ? 'cw-file-folder' : fileTypeClass(nameOf(entry));
}

function iconName(entry: BackendDirent): 'folder' | ReturnType<typeof fileTypeIcon> {
  return entry.kind === 'dir' ? 'folder' : fileTypeIcon(nameOf(entry));
}

function folderSubtitle(entry: BackendDirent, entries: BackendDirent[]): string {
  if (entry.kind !== 'dir') return '';
  const n = countDescendants(entries, entry.path.split('/').filter(Boolean));
  return n === 0 ? '빈 폴더' : `${n}개 항목`;
}

function ListRow({ entry, index, entries, selected, showPath, onSelect, onOpen, onDelete }: RowProps) {
  const isDir = entry.kind === 'dir';
  return (
    <div
      className={`cw-file-row${selected ? ' is-selected' : ''}`}
      role="button"
      tabIndex={0}
      aria-selected={selected}
      data-row-index={index}
      data-row-path={entry.path}
      onClick={(e) => { e.stopPropagation(); onSelect(entry, e); }}
      onDoubleClick={(e) => { e.stopPropagation(); onOpen(entry); }}
      onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); onOpen(entry); } }}
    >
      <span className={`cw-pocket ${iconClass(entry)}`}>
        <Icon name={iconName(entry)} size={14} />
      </span>
      <span className="cw-file-main">
        <span className="name">{nameOf(entry)}</span>
        <span className="meta">
          {showPath && `${entry.path.split('/').slice(0, -1).join('/') || '/'} · `}
          {isDir
            ? folderSubtitle(entry, entries)
            : `${entry.modified_at ? new Date(entry.modified_at).toLocaleDateString() : '—'} · ${formatBytes(entry.bytes ?? 0)}`}
        </span>
      </span>
      <button
        type="button"
        className="cw-file-more"
        aria-label={isDir ? '폴더 삭제' : '파일 삭제'}
        onClick={(e) => { e.stopPropagation(); onDelete(entry); }}
      >
        <Icon name="trash" size={14} />
      </button>
    </div>
  );
}

function GridCard({ entry, index, entries, selected, showPath, onSelect, onOpen, onDelete }: RowProps) {
  const isDir = entry.kind === 'dir';
  return (
    <div
      className={`cw-grid-card${selected ? ' is-selected' : ''}`}
      role="button"
      tabIndex={0}
      aria-selected={selected}
      data-row-index={index}
      data-row-path={entry.path}
      onClick={(e) => { e.stopPropagation(); onSelect(entry, e); }}
      onDoubleClick={(e) => { e.stopPropagation(); onOpen(entry); }}
      onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); onOpen(entry); } }}
    >
      <div className={`cw-grid-card-icon ${iconClass(entry)}`}>
        <Icon name={iconName(entry)} size={28} />
      </div>
      <div className="cw-grid-card-name" title={nameOf(entry)}>{nameOf(entry)}</div>
      <div className="cw-grid-card-meta">
        {showPath
          ? entry.path.split('/').slice(0, -1).join('/') || '/'
          : isDir
            ? folderSubtitle(entry, entries)
            : formatBytes(entry.bytes ?? 0)}
      </div>
      <button
        type="button"
        className="cw-grid-card-more"
        aria-label={isDir ? '폴더 삭제' : '파일 삭제'}
        onClick={(e) => { e.stopPropagation(); onDelete(entry); }}
      >
        <Icon name="trash" size={13} />
      </button>
    </div>
  );
}

// ── Helpers ───────────────────────────────────────────────────────────

function describeBulkDelete(
  pending: BackendDirent[] | null,
  entries: BackendDirent[],
): { title: string; body: string } | null {
  if (!pending || pending.length === 0) return null;
  if (pending.length === 1) {
    const e = pending[0]!;
    const name = nameOf(e);
    if (e.kind === 'dir') {
      const n = countDescendants(entries, e.path.split('/').filter(Boolean));
      return {
        title: '폴더를 삭제하시겠어요?',
        body: n > 0
          ? `${name} 폴더와 그 안의 파일 ${n}개가 함께 삭제됩니다. 되돌릴 수 없습니다.`
          : `${name} 폴더를 삭제합니다. 되돌릴 수 없습니다.`,
      };
    }
    return { title: '파일을 삭제하시겠어요?', body: `${name} 을(를) 삭제하면 되돌릴 수 없습니다.` };
  }
  const folders = pending.filter((e) => e.kind === 'dir');
  const files = pending.filter((e) => e.kind === 'file');
  const parts: string[] = [];
  if (folders.length) parts.push(`폴더 ${folders.length}개`);
  if (files.length) parts.push(`파일 ${files.length}개`);
  let body = `${parts.join(', ')}를 삭제합니다.`;
  if (folders.length > 0) {
    const total = folders.reduce((sum, f) => sum + countDescendants(entries, f.path.split('/').filter(Boolean)), 0);
    if (total > 0) body += ` 폴더 내부 파일 ${total}개도 함께 삭제됩니다.`;
  }
  body += ' 되돌릴 수 없습니다.';
  return { title: `${pending.length}개 항목을 삭제하시겠어요?`, body };
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
