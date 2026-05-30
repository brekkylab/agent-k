// ArtifactsPanel — lists agent-generated artifacts for a session.
// Copy-to-shared is handled by CopyToSharedDialog (passed as a render prop).

import { useEffect, useRef, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { deleteDirent, downloadFile, listDirentsRaw, stripScopePrefix, type DirentScope } from '@/api/dirents';
import { nameOf } from '@/domain/files';
import { FileTypeIcon } from './FileTypeIcon';
import { Icon } from './Icon';
import { EmptyState } from './uiPrimitives';
import { ConfirmDialog } from './ConfirmDialog';
import { useToastStore } from './Toast';

interface ArtifactsPanelProps {
  projectId: string;
  sessionId: string;
  /** Called when user requests "copy to shared" for the given relative paths.
   *  Pass null if CopyToSharedDialog is not yet wired. */
  onCopyToShared?: (relativePaths: string[]) => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function ArtifactsPanel({ projectId, sessionId, onCopyToShared }: ArtifactsPanelProps) {
  const queryClient = useQueryClient();
  const showToast = useToastStore((s) => s.show);

  const scope: DirentScope = { kind: 'artifacts', projectId, sessionId };

  const { data: rawEntries = [], isLoading } = useQuery({
    queryKey: ['dirents', 'artifacts', projectId, sessionId],
    queryFn: () => listDirentsRaw(scope, true),
    enabled: Boolean(sessionId),
  });

  // Show files only, strip scope prefix for display
  const entries = rawEntries
    .filter((e) => e.kind === 'file')
    .map((e) => ({ ...e, path: stripScopePrefix(scope, e.path) }));

  const [collapsed, setCollapsed] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  // 'bulk' is the key for the selection-row menu; individual entries use their path
  const [menuOpen, setMenuOpen] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string[] | null>(null);
  const openMenuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!menuOpen) return;
    function onPointerDown(e: PointerEvent) {
      if (openMenuRef.current && !openMenuRef.current.contains(e.target as Node)) {
        setMenuOpen(null);
      }
    }
    document.addEventListener('pointerdown', onPointerDown);
    return () => document.removeEventListener('pointerdown', onPointerDown);
  }, [menuOpen]);

  useEffect(() => {
    setSelected(prev => {
      if (prev.size === 0) return prev;
      const valid = new Set(entries.map(e => e.path));
      const next = new Set([...prev].filter(p => valid.has(p)));
      return next.size === prev.size ? prev : next;
    });
  }, [entries]);

  const deleteMutation = useMutation({
    mutationFn: async (paths: string[]) => {
      for (const p of paths) await deleteDirent(scope, p);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['dirents', 'artifacts', projectId, sessionId] });
      void queryClient.invalidateQueries({ queryKey: ['messages', sessionId] });
      showToast('삭제되었습니다');
      setSelected(new Set());
      setConfirmDelete(null);
    },
    onError: () => showToast('삭제 실패'),
  });

  const allSelected = entries.length > 0 && entries.every((e) => selected.has(e.path));

  function toggleEntry(path: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  function toggleAll() {
    if (allSelected) {
      setSelected(new Set());
    } else {
      setSelected(new Set(entries.map((e) => e.path)));
    }
  }

  return (
    <div className="cw-artifacts-panel">
      {/* ── collapsible header ─────────────────────────────────── */}
      <button
        type="button"
        className="cw-artifacts-header"
        onClick={() => setCollapsed((c) => !c)}
        aria-expanded={!collapsed}
      >
        <Icon name={collapsed ? 'chevron-right' : 'chevron'} size={12} />
        <span>Artifacts</span>
        {entries.length > 0 && (
          <span className="cw-artifacts-count">{entries.length}</span>
        )}
      </button>

      {!collapsed && (
        <>
          {/* ── select-all row with bulk '...' menu ──────────────── */}
          {entries.length > 0 && (
            <div className="cw-artifacts-selrow">
              <input
                type="checkbox"
                checked={allSelected}
                onChange={toggleAll}
                style={{ cursor: 'pointer' }}
              />
              <span style={{ cursor: 'pointer', userSelect: 'none', flex: 1 }} onClick={toggleAll}>
                {selected.size > 0
                  ? `${selected.size}개 선택됨`
                  : (allSelected ? '전체 해제' : '전체 선택')}
              </span>
              {selected.size > 0 && (
                <div className="cw-artifact-menu-wrap" ref={menuOpen === 'bulk' ? openMenuRef : null}>
                  <button
                    type="button"
                    aria-label="선택 항목 작업"
                    onClick={() => setMenuOpen(menuOpen === 'bulk' ? null : 'bulk')}
                  >
                    <Icon name="more" size={13} />
                  </button>
                  {menuOpen === 'bulk' && (
                    <ul className="cw-file-dropdown" onClick={() => setMenuOpen(null)}>
                      {onCopyToShared && (
                        <li>
                          <button type="button" onClick={() => onCopyToShared([...selected])}>
                            <Icon name="file" size={13} /> 공유 디렉토리로 복사
                          </button>
                        </li>
                      )}
                      <li>
                        <button
                          type="button"
                          className="cw-file-dropdown-destructive"
                          onClick={() => setConfirmDelete([...selected])}
                        >
                          <Icon name="trash" size={13} /> 삭제
                        </button>
                      </li>
                    </ul>
                  )}
                </div>
              )}
            </div>
          )}

          {/* ── file rows ─────────────────────────────────────────── */}
          {entries.map((entry) => (
            <div className="cw-artifact-row" key={entry.path} style={{ cursor: 'pointer' }} onClick={() => toggleEntry(entry.path)}>
              <input
                type="checkbox"
                checked={selected.has(entry.path)}
                onChange={() => toggleEntry(entry.path)}
                onClick={(e) => e.stopPropagation()}
                style={{ cursor: 'pointer', flexShrink: 0 }}
              />
              <span className="cw-artifact-name">
                <FileTypeIcon filename={nameOf(entry)} size={16} />
                {nameOf(entry)}
              </span>
              <span className="cw-artifact-size">{entry.bytes != null ? formatBytes(entry.bytes) : ''}</span>
              <div className="cw-artifact-menu-wrap" ref={menuOpen === entry.path ? openMenuRef : null} onClick={(e) => e.stopPropagation()}>
                <button
                  type="button"
                  aria-label="더보기"
                  onClick={() => setMenuOpen(menuOpen === entry.path ? null : entry.path)}
                >
                  <Icon name="more" size={13} />
                </button>
                {menuOpen === entry.path && (
                  <ul className="cw-file-dropdown" onClick={() => setMenuOpen(null)}>
                    <li>
                      <button type="button" onClick={() => downloadFile(scope, entry.path)}>
                        <Icon name="download" size={13} /> 다운로드
                      </button>
                    </li>
                    {onCopyToShared && (
                      <li>
                        <button type="button" onClick={() => onCopyToShared([entry.path])}>
                          <Icon name="file" size={13} /> 공유 디렉토리로 복사
                        </button>
                      </li>
                    )}
                    <li>
                      <button type="button" className="cw-file-dropdown-destructive" onClick={() => setConfirmDelete([entry.path])}>
                        <Icon name="trash" size={13} /> 삭제
                      </button>
                    </li>
                  </ul>
                )}
              </div>
            </div>
          ))}

          {!isLoading && entries.length === 0 && (
            <EmptyState chip="📦" title="산출물 없음" body="에이전트가 파일을 생성하면 여기에 표시됩니다." />
          )}
        </>
      )}

      {confirmDelete !== null && (
        <ConfirmDialog
          title="삭제 확인"
          body={`${confirmDelete.length}개 파일을 삭제하시겠습니까?`}
          confirmLabel="삭제"
          destructive
          pending={deleteMutation.isPending}
          onConfirm={() => deleteMutation.mutate(confirmDelete)}
          onClose={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}
