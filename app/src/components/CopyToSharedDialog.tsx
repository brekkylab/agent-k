// Dialog for copying artifact files into the shared folder.
// Mirrors FolderPickerDialog — shows the shared folder tree and lets the user
// pick a destination folder before confirming the copy.

import { useMemo, useRef, useState, useEffect } from 'react';
import { createPortal } from 'react-dom';
import { useQuery, useMutation } from '@tanstack/react-query';
import { copyDirents, listDirentsRaw, stripScopePrefix, type DirentScope } from '@/api/dirents';
import { buildFolderTree, type FolderNode } from '@/domain/files';
import { Icon } from './Icon';
import { useToastStore } from './Toast';

interface Props {
  open: boolean;
  projectId: string;
  sessionId: string;
  sourcePaths: string[];      // scope-relative paths under artifacts/
  onClose: () => void;
  onDone: () => void;
}

function PickerNode({
  node,
  depth,
  expanded,
  selected,
  disabledPaths,
  onToggle,
  onSelect,
}: {
  node: FolderNode;
  depth: number;
  expanded: Set<string>;
  selected: string | null;
  disabledPaths: Set<string>;
  onToggle: (path: string) => void;
  onSelect: (path: string) => void;
}) {
  const hasChildren = node.children.length > 0;
  const isOpen = expanded.has(node.path);
  const isSelected = selected === node.path;

  let isDisabled = false;
  for (const dp of disabledPaths) {
    if (node.path === dp || node.path.startsWith(dp + '/')) {
      isDisabled = true;
      break;
    }
  }

  return (
    <>
      <div
        className={`cw-tree-row${isSelected ? ' is-selected' : ''}${isDisabled ? ' is-disabled' : ''}`}
        style={{ paddingLeft: 2 + depth * 8 }}
      >
        {hasChildren ? (
          <button
            type="button"
            className="cw-tree-chevron"
            aria-label={isOpen ? '접기' : '펼치기'}
            onClick={(e) => { e.stopPropagation(); onToggle(node.path); }}
          >
            <Icon name={isOpen ? 'chevron' : 'chevron-right'} size={12} />
          </button>
        ) : null}
        <button
          type="button"
          className="cw-tree-label"
          onClick={() => { if (!isDisabled) onSelect(node.path); }}
          disabled={isDisabled}
        >
          <Icon name={isOpen ? 'folder-open' : 'folder'} size={14} />
          <span>{node.name}</span>
        </button>
      </div>
      {isOpen && node.children.map((child) => (
        <PickerNode
          key={child.path}
          node={child}
          depth={depth + 1}
          expanded={expanded}
          selected={selected}
          disabledPaths={disabledPaths}
          onToggle={onToggle}
          onSelect={onSelect}
        />
      ))}
    </>
  );
}

export function CopyToSharedDialog({ open, projectId, sessionId, sourcePaths, onClose, onDone }: Props) {
  const showToast = useToastStore((s) => s.show);

  const sharedScope: DirentScope = { kind: 'shared', projectId };
  const artifactsScope: DirentScope = { kind: 'artifacts', projectId, sessionId };

  const entries = useQuery({
    queryKey: ['dirents', 'shared', projectId],
    queryFn: () => listDirentsRaw(sharedScope, true),
    enabled: open,
  });

  const sharedEntries = useMemo(
    () => (entries.data ?? []).map((e) => ({ ...e, path: stripScopePrefix(sharedScope, e.path) })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [entries.data, projectId],
  );

  const tree = useMemo(() => buildFolderTree(sharedEntries), [sharedEntries]);

  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [selected, setSelected] = useState<string>('');

  const copyMutation = useMutation({
    mutationFn: (dest: string) => copyDirents(artifactsScope, sharedScope, sourcePaths, dest),
    onSuccess: () => {
      showToast('공유 폴더로 복사되었습니다');
      onDone();
      onClose();
    },
    onError: () => showToast('복사 실패'),
  });

  const pending = copyMutation.isPending;

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape' && !pending) onClose();
      if (e.key === 'Enter' && !pending) {
        e.preventDefault();
        copyMutation.mutate(selected);
      }
    }
    if (open) {
      window.addEventListener('keydown', onKey);
      return () => window.removeEventListener('keydown', onKey);
    }
  }, [open, onClose, copyMutation, pending, selected]);

  const downOnBackdropRef = useRef(false);

  function toggleExpand(path: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  const subtitle = sourcePaths.length === 1
    ? `"${sourcePaths[0]!.split('/').pop()}"을(를) 공유 폴더로 복사합니다.`
    : `${sourcePaths.length}개 파일을 공유 폴더로 복사합니다.`;

  if (!open) return null;

  return createPortal(
    <div
      className="cw-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      onMouseDown={(e) => { downOnBackdropRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
        const wasDown = downOnBackdropRef.current;
        downOnBackdropRef.current = false;
        if (wasDown && e.target === e.currentTarget && !pending) onClose();
      }}
    >
      <div className="cw-dialog">
        <button type="button" className="cw-close" onClick={onClose} disabled={pending} aria-label="close">
          <Icon name="x" />
        </button>
        <h2 style={{ margin: '0 0 6px', fontSize: 18, letterSpacing: '-0.015em' }}>공유 폴더로 복사</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 12px', fontSize: 13, lineHeight: 1.55 }}>
          {subtitle}
        </p>
        <div className="cw-folder-picker-tree">
          {entries.isLoading ? (
            <p style={{ padding: '8px', color: 'var(--cw-ink-3)', fontSize: 13 }}>Loading…</p>
          ) : (
            <>
              <div
                className={`cw-tree-row${selected === '' ? ' is-selected' : ''}`}
                style={{ paddingLeft: 2 }}
              >
                <button type="button" className="cw-tree-label" onClick={() => setSelected('')}>
                  <Icon name="folder" size={14} />
                  <span>/</span>
                </button>
              </div>
              {tree.map((node) => (
                <PickerNode
                  key={node.path}
                  node={node}
                  depth={1}
                  expanded={expanded}
                  selected={selected}
                  disabledPaths={new Set()}
                  onToggle={toggleExpand}
                  onSelect={setSelected}
                />
              ))}
            </>
          )}
        </div>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', marginTop: 18 }}>
          <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>
            취소
          </button>
          <button
            type="button"
            className="cw-btn-primary"
            disabled={pending}
            onClick={() => copyMutation.mutate(selected)}
          >
            {pending ? '복사 중…' : '복사'}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
