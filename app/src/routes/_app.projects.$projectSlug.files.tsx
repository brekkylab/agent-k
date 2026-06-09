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
import { useTranslation } from 'react-i18next';
import { localizedNoun } from '@/i18n';
import { loadNs } from '@/i18n/loader';
import type { TFunction } from 'i18next';
import {
  copyDirents,
  createFolder,
  deleteDirent,
  downloadFile,
  listDirentsRaw,
  moveDirents,
  uploadFiles,
  type DirentBatchResult,
  type DirentScope,
  stripScopePrefix,
} from '@/api/dirents';
import { getProject } from '@/api/projects';
import { Icon } from '@/components/Icon';
import { FileTypeIcon } from '@/components/FileTypeIcon';
import { ConfirmDialog } from '@/components/ConfirmDialog';
import { FolderPickerDialog } from '@/components/FolderPickerDialog';
import { NewFolderDialog } from '@/components/NewFolderDialog';
import { RenameDialog } from '@/components/RenameDialog';
import { EmptyState, IconPocket } from '@/components/uiPrimitives';
import { useToastStore } from '@/components/Toast';
import { ApiError } from '@/api/client';
import {
  ancestorPaths,
  buildFolderTree,
  countDescendants,
  listDirectChildren,
  nameOf,
  type FolderNode,
} from '@/domain/files';
import type { BackendDirent } from '@/api/backend-types';

type ViewMode = 'list' | 'grid';
const VIEW_KEY = 'cowork.files.viewMode';
const DRAG_THRESHOLD = 5; // px — under this we treat mousedown as click

export const Route = createFileRoute('/_app/projects/$projectSlug/files')({
  // Rename / NewFolder / CopyToShared / FolderPicker all live on this page → `dialogs`.
  loader: () => loadNs('files', 'dialogs'),
  component: FilesPage,
});

function FilesPage() {
  const { projectSlug } = Route.useParams();
  const { t, i18n } = useTranslation(['files', 'common']);
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const project = useQuery({ queryKey: ['project', projectSlug], queryFn: () => getProject(projectSlug) });
  // Dirents are scope-based, keyed by the resolved project UUID (not the slug).
  const projectId = project.data?.id ?? '';
  const scope: DirentScope = { kind: 'shared', projectId };
  const dirents = useQuery({
    queryKey: ['dirents', 'shared', projectId],
    queryFn: () => listDirentsRaw(scope),
    select: (raw) => raw.map((e) => ({ ...e, path: stripScopePrefix(scope, e.path) })),
    enabled: Boolean(project.data),
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
    mutationFn: async (files: File[]): Promise<DirentBatchResult> => {
      const items = files.map((file) => ({ file, targetPath: targetPathFor(file) }));
      return uploadFiles(scope, items);
    },
    onSuccess: async (result) => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', 'shared', projectId] });
      const ok = result.succeeded.length;
      const ko = result.failed.length;
      if (ko === 0) {
        showToast(t('files:toast.upload_success', { count: ok }));
      } else {
        showToast(
          t('files:toast.upload_partial', { ok, ko }),
          result.failed.map((f) => t('files:toast.failed_entry', { path: f.path || t('files:toast.unnamed'), error: f.error })),
        );
      }
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'upload failed';
      showToast(t('files:toast.upload_failed', { message: msg }));
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
      return createFolder(scope, fullPath);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', 'shared', projectId] });
      showToast(t('files:toast.folder_created'));
      setFolderDialogOpen(false);
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'mkdir failed';
      showToast(t('files:toast.folder_failed', { message: msg }));
    },
  });

  const downloadMutation = useMutation({
    mutationFn: (entry: BackendDirent) => downloadFile(scope, entry.path),
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'download failed';
      showToast(t('files:toast.download_failed', { message: msg }));
    },
  });

  const [pendingDelete, setPendingDelete] = useState<BackendDirent[] | null>(null);
  const [pendingRename, setPendingRename] = useState<BackendDirent | null>(null);
  const [pendingMove, setPendingMove] = useState<BackendDirent[] | null>(null);
  const [pendingCopy, setPendingCopy] = useState<BackendDirent[] | null>(null);
  const [openMenuPath, setOpenMenuPath] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  // Disambiguates "rename via dialog" from generic moves so we can show the
  // right toast copy ("the name was changed" vs "moved").
  const renamingRef = useRef(false);

  const bulkDeleteMutation = useMutation({
    mutationFn: async (targets: BackendDirent[]) => {
      const results = await Promise.allSettled(targets.map((t) => deleteDirent(scope, t.path)));
      return { targets, results };
    },
    onSuccess: async ({ targets, results }) => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', 'shared', projectId] });
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
        showToast(t('files:toast.delete_success', { count: okCount }));
      } else {
        showToast(
          t('files:toast.delete_partial', { ok: okCount, ko: fails.length }),
          fails.map((f) => t('files:toast.failed_entry', { path: f.path, error: f.error })),
        );
      }
      setSelectedPaths(new Set());
      anchorRef.current = null;
      setPendingDelete(null);
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'delete failed';
      showToast(t('files:toast.delete_failed', { message: msg }));
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
    if (files.length === 0) { showToast(t('files:toast.no_files_selected')); return; }
    files.forEach((f) => downloadMutation.mutate(f));
  }, [selectedEntries, downloadMutation, showToast, t]);

  // Single unified mutation for both rename (1-item move with new_name) and
  // bulk move. The `renamingRef` flag tells the success/error handler whether
  // to show the "the name was changed" or "moved" copy.
  const moveMutation = useMutation({
    mutationFn: ({ sources, destination, newName }: { sources: string[]; destination: string; newName?: string }) =>
      moveDirents(scope, sources, destination, newName),
    onSuccess: async (res) => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', 'shared', projectId] });
      const ok = res.succeeded.length;
      const ko = res.failed.length;
      const wasRename = renamingRef.current;
      renamingRef.current = false;
      if (wasRename) {
        if (ko === 0) showToast(t('files:toast.rename_success'));
        else showToast(t('files:toast.rename_failed'), res.failed.map((f) => f.error));
      } else {
        if (ko === 0) showToast(t('files:toast.move_success', { count: ok }));
        else showToast(
          t('files:toast.move_partial', { ok, ko }),
          res.failed.map((f) => t('files:toast.failed_entry', { path: f.path, error: f.error })),
        );
      }
      setPendingRename(null);
      setPendingMove(null);
      setSelectedPaths(new Set());
      anchorRef.current = null;
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'move failed';
      const wasRename = renamingRef.current;
      renamingRef.current = false;
      const action = t(wasRename ? 'files:toast.rename_action' : 'files:toast.move_action');
      showToast(t('files:toast.rename_or_move_failed', { action, message: msg }));
    },
  });

  const copyMutation = useMutation({
    mutationFn: ({ sources, destination }: { sources: string[]; destination: string }) =>
      copyDirents(scope, scope, sources, destination),
    onSuccess: async (res) => {
      await queryClient.invalidateQueries({ queryKey: ['dirents', 'shared', projectId] });
      const ok = res.succeeded.length;
      const ko = res.failed.length;
      if (ko === 0) showToast(t('files:toast.copy_success', { count: ok }));
      else showToast(
        t('files:toast.copy_partial', { ok, ko }),
        res.failed.map((f) => t('files:toast.failed_entry', { path: f.path, error: f.error })),
      );
      setPendingCopy(null);
      setSelectedPaths(new Set());
      anchorRef.current = null;
    },
    onError: (err) => {
      const msg = err instanceof ApiError ? err.message : err instanceof Error ? err.message : 'copy failed';
      showToast(t('files:toast.copy_failed', { message: msg }));
    },
  });

  // ── Outside click clears selection ───────────────────────────────
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

  // ── Outside click closes ⋯ dropdown ──────────────────────────────
  useEffect(() => {
    if (!openMenuPath) return;
    function onDocClick(e: MouseEvent) {
      const t = e.target as HTMLElement | null;
      if (t?.closest('.cw-file-menu-wrap')) return;
      setOpenMenuPath(null);
    }
    document.addEventListener('click', onDocClick, true);
    return () => document.removeEventListener('click', onDocClick, true);
  }, [openMenuPath]);

  // ── Keyboard shortcuts (window-level) ────────────────────────────
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      const inField = !!target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);

      if (e.key === 'Escape') { if (!inField) { setOpenMenuPath(null); clearSelection(); } return; }
      if (inField) return;

      if (e.key === 'F2' && selectedPaths.size === 1 && !pendingRename) {
        e.preventDefault();
        const entry = entries.find((en) => selectedPaths.has(en.path));
        if (entry) setPendingRename(entry);
        return;
      }

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

  // ── Intra-app drag (move/copy) ───────────────────────────────────
  useEffect(() => {
    function onEnd() { setDropTarget(null); }
    window.addEventListener('dragend', onEnd);
    return () => window.removeEventListener('dragend', onEnd);
  }, []);

  const handleDragStart = useCallback((e: React.DragEvent, entry: BackendDirent) => {
    dragOriginRef.current = null;
    setDragRect(null);
    e.dataTransfer.effectAllowed = 'copyMove';
    const dragPaths = selectedPaths.has(entry.path)
      ? Array.from(selectedPaths)
      : [entry.path];
    e.dataTransfer.setData('application/x-cowork-dirent-paths', JSON.stringify(dragPaths));
    e.dataTransfer.setData('text/plain', dragPaths.join('\n'));

    const ghost = document.createElement('div');
    ghost.className = 'cw-drag-ghost';
    ghost.textContent = dragPaths.length === 1
      ? nameOf(entry)
      : t('files:ui.drag_items', { count: dragPaths.length });
    document.body.appendChild(ghost);
    e.dataTransfer.setDragImage(ghost, 14, 14);
    requestAnimationFrame(() => ghost.remove());
  }, [selectedPaths]);

  const handleDropOnFolder = useCallback((destination: string, e: React.DragEvent): boolean => {
    const raw = e.dataTransfer.getData('application/x-cowork-dirent-paths');
    if (!raw) return false;
    let sources: string[];
    try { sources = JSON.parse(raw); } catch { return false; }
    if (sources.some((s) => destination === s || destination.startsWith(s + '/'))) {
      showToast(t('files:toast.invalid_move'));
      return true;
    }
    const isCopy = e.altKey || e.ctrlKey || e.metaKey;
    if (isCopy) copyMutation.mutate({ sources, destination });
    else moveMutation.mutate({ sources, destination });
    return true;
  }, [showToast, copyMutation, moveMutation]);

  const deleteCopy = useMemo(
    () => describeBulkDelete(pendingDelete, entries, t, i18n.language),
    [pendingDelete, entries, t, i18n.language],
  );
  const currentPathKey = currentPath.join('/');

  return (
    <section className="cw-page cw-files-page cw-page-enter">
      <div className="cw-page-head">
        <div>
          <h1>{t('files:page.title')}</h1>
          <p>{t('files:page.description')}</p>
        </div>
        <div>
          <button className="cw-btn-secondary" onClick={openFolderDialog} disabled={folderMutation.isPending}>
            <IconPocket tone="add" icon="plus" compact /> {t('files:ui.new_folder')}
          </button>
          <button className="cw-btn-primary" onClick={openUpload} disabled={uploadMutation.isPending}>
            <IconPocket tone="add" icon="upload" compact /> {uploadMutation.isPending ? t('files:ui.uploading') : t('files:ui.upload')}
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
            className={`cw-tree-row cw-tree-root${currentPath.length === 0 ? ' is-active' : ''}${dropTarget === '' ? ' is-drop-target' : ''}`}
            onClick={() => setCurrentPath([])}
            onDragEnter={(e) => {
              if (!e.dataTransfer.types.includes('application/x-cowork-dirent-paths')) return;
              e.preventDefault();
              setDropTarget('');
            }}
            onDragOver={(e) => {
              if (!e.dataTransfer.types.includes('application/x-cowork-dirent-paths')) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = e.altKey || e.ctrlKey || e.metaKey ? 'copy' : 'move';
              setDropTarget('');
            }}
            onDragLeave={(e) => {
              const to = e.relatedTarget as Node | null;
              if (!to || !e.currentTarget.contains(to)) setDropTarget(null);
            }}
            onDrop={(e) => {
              if (!handleDropOnFolder('', e)) return;
              e.preventDefault();
              setDropTarget(null);
            }}
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
              dropTarget={dropTarget}
              onSelect={(segments) => setCurrentPath(segments)}
              onToggle={toggleExpand}
              onDropOnFolder={handleDropOnFolder}
              setDropTarget={setDropTarget}
            />
          ))}
        </aside>

        <section className="cw-file-pane">
          <header className="cw-file-pane-head">
            {selectedPaths.size >= 1 ? (
              <div className="cw-bulk-toolbar cw-bulk-toolbar-inline" role="toolbar" aria-label="bulk actions">
                <span className="cw-bulk-count">{t('files:ui.selected_count', { count: selectedPaths.size })}</span>
                <button type="button" className="cw-btn-secondary" onClick={bulkDownload}>
                  <Icon name="download" size={13} /> {t('files:ui.download')}
                </button>
                {selectedPaths.size === 1 && (
                  <button type="button" className="cw-btn-secondary" onClick={() => setPendingRename(selectedEntries[0]!)}>
                    <Icon name="writing" size={13} /> {t('files:ui.rename')}
                  </button>
                )}
                <button type="button" className="cw-btn-secondary" onClick={() => setPendingMove(selectedEntries)}>
                  <Icon name="chevron-right" size={13} /> {t('files:ui.move')}
                </button>
                <button type="button" className="cw-btn-secondary" onClick={() => setPendingCopy(selectedEntries)}>
                  <Icon name="file" size={13} /> {t('files:ui.copy')}
                </button>
                <button type="button" className="cw-btn-secondary cw-btn-destructive" onClick={requestBulkDelete}>
                  <Icon name="trash" size={13} /> {t('files:ui.delete')}
                </button>
                <button type="button" className="cw-bulk-clear" onClick={clearSelection} aria-label={t('files:ui.clear_selection')}>
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
                <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t('files:ui.search_placeholder')} />
              </label>
              <div className="cw-view-toggle" role="tablist" aria-label={t('files:ui.view_toggle_label')}>
                <button type="button" role="tab" aria-selected={viewMode === 'list'} className={viewMode === 'list' ? 'is-active' : ''} onClick={() => setViewMode('list')} aria-label={t('files:ui.list_view')}><Icon name="list" size={14} /></button>
                <button type="button" role="tab" aria-selected={viewMode === 'grid'} className={viewMode === 'grid' ? 'is-active' : ''} onClick={() => setViewMode('grid')} aria-label={t('files:ui.grid_view')}><Icon name="grid" size={14} /></button>
              </div>
            </div>
          </header>

          <div
            ref={bodyRef}
            className={`cw-file-body cw-view-${viewMode}${isDraggingOver ? ' is-over' : ''}`}
            onClick={(e) => { if (e.target === e.currentTarget) clearSelection(); }}
            onMouseDown={onBodyMouseDown}
            onDragEnter={(e) => {
              if (e.dataTransfer.types.includes('application/x-cowork-dirent-paths')) return;
              e.preventDefault();
              dragDepthRef.current += 1;
              setIsDraggingOver(true);
            }}
            onDragOver={(e) => { e.preventDefault(); }}
            onDragLeave={(e) => {
              if (e.dataTransfer.types.includes('application/x-cowork-dirent-paths')) return;
              e.preventDefault();
              dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
              if (dragDepthRef.current === 0) setIsDraggingOver(false);
            }}
            onDrop={(e) => {
              e.preventDefault();
              dragDepthRef.current = 0;
              setIsDraggingOver(false);
              if (e.dataTransfer.types.includes('application/x-cowork-dirent-paths')) return;
              onDropFiles(e.dataTransfer.files);
            }}
          >
            {allRows.length === 0 ? (
              <EmptyState
                title={view.isSearch ? t('files:empty.search_no_results') : t('files:empty.folder_empty_title')}
                body={view.isSearch
                  ? t('files:empty.search_hint')
                  : t('files:empty.folder_empty_hint')}
                action={view.isSearch ? undefined : t('files:empty_action.upload')}
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
                    menuOpen={openMenuPath === entry.path}
                    onSelect={handleSelect}
                    onOpen={openEntry}
                    onDownload={(e) => downloadMutation.mutate(e)}
                    onDelete={(e) => setPendingDelete([e])}
                    onRename={(e) => setPendingRename(e)}
                    onMove={(e) => setPendingMove([e])}
                    onCopy={(e) => setPendingCopy([e])}
                    onMenuToggle={setOpenMenuPath}
                    onDragStart={handleDragStart}
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
                    menuOpen={openMenuPath === entry.path}
                    onSelect={handleSelect}
                    onOpen={openEntry}
                    onDownload={(e) => downloadMutation.mutate(e)}
                    onDelete={(e) => setPendingDelete([e])}
                    onRename={(e) => setPendingRename(e)}
                    onMove={(e) => setPendingMove([e])}
                    onCopy={(e) => setPendingCopy([e])}
                    onMenuToggle={setOpenMenuPath}
                    onDragStart={handleDragStart}
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
          confirmLabel={t('files:delete.confirm')}
          destructive
          pending={bulkDeleteMutation.isPending}
          onConfirm={() => bulkDeleteMutation.mutate(pendingDelete)}
          onClose={() => { if (!bulkDeleteMutation.isPending) setPendingDelete(null); }}
        />
      )}

      {pendingRename && (
        <RenameDialog
          entry={pendingRename}
          existingNames={view.folders.concat(view.files).map(nameOf)}
          pending={moveMutation.isPending}
          onConfirm={(newName) => {
            const parent = pendingRename.path.split('/').slice(0, -1).join('/');
            renamingRef.current = true;
            moveMutation.mutate({ sources: [pendingRename.path], destination: parent, newName });
          }}
          onClose={() => { if (!moveMutation.isPending) setPendingRename(null); }}
        />
      )}

      {pendingMove && (
        <FolderPickerDialog
          title={t('files:ops.move_title')}
          confirmLabel={t('files:ops.move_confirm')}
          entries={entries}
          sources={pendingMove}
          pending={moveMutation.isPending}
          onConfirm={(destination) => moveMutation.mutate({
            sources: pendingMove.map((e) => e.path),
            destination,
          })}
          onClose={() => { if (!moveMutation.isPending) setPendingMove(null); }}
        />
      )}

      {pendingCopy && (
        <FolderPickerDialog
          title={t('files:ops.copy_title')}
          confirmLabel={t('files:ops.copy_confirm')}
          entries={entries}
          sources={pendingCopy}
          pending={copyMutation.isPending}
          onConfirm={(destination) => copyMutation.mutate({
            sources: pendingCopy.map((e) => e.path),
            destination,
          })}
          onClose={() => { if (!copyMutation.isPending) setPendingCopy(null); }}
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
  dropTarget,
  onSelect,
  onToggle,
  onDropOnFolder,
  setDropTarget,
}: {
  node: FolderNode;
  depth: number;
  currentPathKey: string;
  expanded: Set<string>;
  entries: BackendDirent[];
  dropTarget: string | null;
  onSelect: (segments: string[]) => void;
  onToggle: (path: string) => void;
  onDropOnFolder: (destination: string, e: React.DragEvent) => boolean;
  setDropTarget: (path: string | null) => void;
}) {
  const { t } = useTranslation('files');
  const isActive = currentPathKey === node.path;
  const hasChildren = node.children.length > 0;
  const isOpen = expanded.has(node.path);
  const count = countDescendants(entries, node.segments);
  const isDropTarget = dropTarget === node.path;

  return (
    <>
      <div
        className={`cw-tree-row${isActive ? ' is-active' : ''}${isDropTarget ? ' is-drop-target' : ''}`}
        style={{ paddingLeft: 8 + depth * 14 }}
        onDragEnter={(e) => {
          if (!e.dataTransfer.types.includes('application/x-cowork-dirent-paths')) return;
          e.preventDefault();
          e.stopPropagation();
          setDropTarget(node.path);
        }}
        onDragOver={(e) => {
          if (!e.dataTransfer.types.includes('application/x-cowork-dirent-paths')) return;
          e.preventDefault();
          e.stopPropagation();
          e.dataTransfer.dropEffect = e.altKey || e.ctrlKey || e.metaKey ? 'copy' : 'move';
          setDropTarget(node.path);
        }}
        onDragLeave={(e) => {
          const to = e.relatedTarget as Node | null;
          if (!to || !e.currentTarget.contains(to)) setDropTarget(null);
        }}
        onDrop={(e) => {
          if (!onDropOnFolder(node.path, e)) return;
          e.preventDefault();
          e.stopPropagation();
          setDropTarget(null);
        }}
      >
        <button
          type="button"
          className="cw-tree-chevron"
          aria-label={isOpen ? t('ui.collapse') : t('ui.expand')}
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
          dropTarget={dropTarget}
          onSelect={onSelect}
          onToggle={onToggle}
          onDropOnFolder={onDropOnFolder}
          setDropTarget={setDropTarget}
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
  menuOpen: boolean;
  onSelect: (e: BackendDirent, ev: React.MouseEvent | React.KeyboardEvent) => void;
  onOpen: (e: BackendDirent) => void;
  onDownload: (e: BackendDirent) => void;
  onDelete: (e: BackendDirent) => void;
  onRename: (e: BackendDirent) => void;
  onMove: (e: BackendDirent) => void;
  onCopy: (e: BackendDirent) => void;
  onMenuToggle: (path: string | null) => void;
  onDragStart: (ev: React.DragEvent, e: BackendDirent) => void;
}


function folderSubtitle(entry: BackendDirent, entries: BackendDirent[], t: TFunction<'files'>): string {
  if (entry.kind !== 'dir') return '';
  const n = countDescendants(entries, entry.path.split('/').filter(Boolean));
  return n === 0 ? t('ui.empty_folder') : t('ui.item_count', { count: n });
}

function RowMenu({
  entry, menuOpen, onMenuToggle, onDownload, onRename, onMove, onCopy, onDelete,
}: {
  entry: BackendDirent;
  menuOpen: boolean;
  onMenuToggle: (path: string | null) => void;
  onDownload: () => void;
  onRename: () => void;
  onMove: () => void;
  onCopy: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation('files');
  return (
    <div className="cw-file-menu-wrap">
      <button
        type="button"
        className="cw-file-more"
        aria-label={t('ui.more')}
        aria-expanded={menuOpen}
        onClick={(e) => { e.stopPropagation(); onMenuToggle(menuOpen ? null : entry.path); }}
      >
        <Icon name="more" size={14} />
      </button>
      {menuOpen && (
        <ul
          className="cw-file-dropdown"
          role="menu"
          onClick={(e) => e.stopPropagation()}
        >
          {entry.kind === 'file' && (
            <li role="menuitem">
              <button type="button" onClick={() => { onMenuToggle(null); onDownload(); }}>
                <Icon name="download" size={13} /> {t('ui.download')}
              </button>
            </li>
          )}
          <li role="menuitem">
            <button type="button" onClick={() => { onMenuToggle(null); onRename(); }}>
              <Icon name="writing" size={13} /> {t('ui.rename')}
            </button>
          </li>
          <li role="menuitem">
            <button type="button" onClick={() => { onMenuToggle(null); onMove(); }}>
              <Icon name="chevron-right" size={13} /> {t('ui.move_dropdown')}
            </button>
          </li>
          <li role="menuitem">
            <button type="button" onClick={() => { onMenuToggle(null); onCopy(); }}>
              <Icon name="file" size={13} /> {t('ui.copy_dropdown')}
            </button>
          </li>
          <li role="separator" aria-hidden="true" className="cw-file-dropdown-sep" />
          <li role="menuitem">
            <button type="button" className="cw-file-dropdown-destructive" onClick={() => { onMenuToggle(null); onDelete(); }}>
              <Icon name="trash" size={13} /> {t('ui.delete')}
            </button>
          </li>
        </ul>
      )}
    </div>
  );
}

function ListRow({ entry, index, entries, selected, showPath, menuOpen, onSelect, onOpen, onDownload, onDelete, onRename, onMove, onCopy, onMenuToggle, onDragStart }: RowProps) {
  const { t } = useTranslation('files');
  const isDir = entry.kind === 'dir';
  return (
    <div
      className={`cw-file-row${selected ? ' is-selected' : ''}`}
      role="button"
      tabIndex={0}
      aria-selected={selected}
      data-row-index={index}
      data-row-path={entry.path}
      draggable
      onDragStart={(e) => onDragStart(e, entry)}
      onClick={(e) => { e.stopPropagation(); onSelect(entry, e); }}
      onDoubleClick={(e) => { e.stopPropagation(); onOpen(entry); }}
      onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); onOpen(entry); } }}
    >
      {isDir
        ? <span className="cw-pocket cw-file-folder"><Icon name="folder" size={14} /></span>
        : <FileTypeIcon filename={nameOf(entry)} size={18} />
      }
      <span className="cw-file-main">
        <span className="name">{nameOf(entry)}</span>
        <span className="meta">
          {showPath && `${entry.path.split('/').slice(0, -1).join('/') || '/'} · `}
          {isDir
            ? folderSubtitle(entry, entries, t)
            : `${entry.modified_at ? new Date(entry.modified_at).toLocaleDateString() : '—'} · ${formatBytes(entry.bytes ?? 0)}`}
        </span>
      </span>
      <RowMenu
        entry={entry}
        menuOpen={menuOpen}
        onMenuToggle={onMenuToggle}
        onDownload={() => onDownload(entry)}
        onRename={() => onRename(entry)}
        onMove={() => onMove(entry)}
        onCopy={() => onCopy(entry)}
        onDelete={() => onDelete(entry)}
      />
    </div>
  );
}

function GridCard({ entry, index, entries, selected, showPath, menuOpen, onSelect, onOpen, onDownload, onDelete, onRename, onMove, onCopy, onMenuToggle, onDragStart }: RowProps) {
  const { t } = useTranslation('files');
  const isDir = entry.kind === 'dir';
  return (
    <div
      className={`cw-grid-card${selected ? ' is-selected' : ''}`}
      role="button"
      tabIndex={0}
      aria-selected={selected}
      data-row-index={index}
      data-row-path={entry.path}
      draggable
      onDragStart={(e) => onDragStart(e, entry)}
      onClick={(e) => { e.stopPropagation(); onSelect(entry, e); }}
      onDoubleClick={(e) => { e.stopPropagation(); onOpen(entry); }}
      onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); onOpen(entry); } }}
    >
      {isDir
        ? <div className="cw-grid-card-icon cw-file-folder"><Icon name="folder" size={28} /></div>
        : <div className="cw-grid-card-icon cw-grid-card-icon--file"><FileTypeIcon filename={nameOf(entry)} size={44} /></div>
      }
      <div className="cw-grid-card-name" title={nameOf(entry)}>{nameOf(entry)}</div>
      <div className="cw-grid-card-meta">
        {showPath
          ? entry.path.split('/').slice(0, -1).join('/') || '/'
          : isDir
            ? folderSubtitle(entry, entries, t)
            : formatBytes(entry.bytes ?? 0)}
      </div>
      <RowMenu
        entry={entry}
        menuOpen={menuOpen}
        onMenuToggle={onMenuToggle}
        onDownload={() => onDownload(entry)}
        onRename={() => onRename(entry)}
        onMove={() => onMove(entry)}
        onCopy={() => onCopy(entry)}
        onDelete={() => onDelete(entry)}
      />
    </div>
  );
}

// ── Helpers ───────────────────────────────────────────────────────────

function describeBulkDelete(
  pending: BackendDirent[] | null,
  entries: BackendDirent[],
  t: TFunction<'files'>,
  lang: string,
): { title: string; body: string } | null {
  if (!pending || pending.length === 0) return null;
  if (pending.length === 1) {
    const e = pending[0]!;
    const name = nameOf(e);
    if (e.kind === 'dir') {
      const n = countDescendants(entries, e.path.split('/').filter(Boolean));
      return {
        title: t('delete.folder_title'),
        body: n > 0
          ? t('delete.folder_body_with_children', { name, count: n })
          : t('delete.folder_body_empty', { name }),
      };
    }
    const decorated = localizedNoun(name, '을/를', lang);
    return {
      title: t('delete.file_title'),
      body: t('delete.file_body', { name: decorated }),
    };
  }
  const folders = pending.filter((e) => e.kind === 'dir');
  const files = pending.filter((e) => e.kind === 'file');
  const parts: string[] = [];
  if (folders.length) parts.push(t('delete.summary_folders', { count: folders.length }));
  if (files.length) parts.push(t('delete.summary_files', { count: files.length }));
  let body = t('delete.bulk_body_summary', { summary: parts.join(', ') });
  if (folders.length > 0) {
    const total = folders.reduce((sum, f) => sum + countDescendants(entries, f.path.split('/').filter(Boolean)), 0);
    if (total > 0) body += t('delete.bulk_body_includes', { count: total });
  }
  body += t('delete.bulk_body_undo');
  return { title: t('delete.bulk_title', { count: pending.length }), body };
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
