// Tree picker dialog for choosing a destination folder for move/copy ops.
// Mirrors the NewFolderDialog visual pattern but renders a recursive folder
// tree built from the project's dirents instead of a text input.
//
// destination convention: "" (empty string) = project root.

import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { buildFolderTree, nameOf, type FolderNode } from '@/domain/files';
import { localizedNoun } from '@/i18n';
import type { BackendDirent } from '@/api/backend-types';
import { useDialogEscape } from '@/lib/useDialogEscape';

interface FolderPickerDialogProps {
  title: string;
  confirmLabel: string;
  entries: BackendDirent[];
  sources: BackendDirent[];
  pending: boolean;
  onConfirm: (destination: string) => void;
  onClose: () => void;
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
  const { t } = useTranslation('dialogs');
  const hasChildren = node.children.length > 0;
  const isOpen = expanded.has(node.path);
  const isSelected = selected === node.path;

  // Disabled if this node is a source folder or any of its descendants.
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
        style={{ paddingLeft: 8 + depth * 14 }}
      >
        <button
          type="button"
          className="cw-tree-chevron"
          aria-label={isOpen ? t('folder_picker.collapse') : t('folder_picker.expand')}
          onClick={(e) => { e.stopPropagation(); onToggle(node.path); }}
          style={{ visibility: hasChildren ? 'visible' : 'hidden' }}
        >
          <Icon name={isOpen ? 'chevron' : 'chevron-right'} size={12} />
        </button>
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

export function FolderPickerDialog({
  title, confirmLabel, entries, sources, pending, onConfirm, onClose,
}: FolderPickerDialogProps) {
  const { t, i18n } = useTranslation('dialogs');
  const { t: tCommon } = useTranslation('common');
  const tree = useMemo(() => buildFolderTree(entries), [entries]);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [selected, setSelected] = useState<string | null>('');

  // Source folders block themselves and their descendants as destinations.
  const disabledPaths = useMemo(() => {
    const s = new Set<string>();
    sources.forEach((src) => {
      if (src.kind === 'dir') s.add(src.path);
    });
    return s;
  }, [sources]);

  // ESC goes through the modal stack so it wins over the files page's own
  // selection-clear handler when this picker is opened from there.
  useDialogEscape(onClose, { disabled: pending });
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Enter' && selected !== null && !pending) {
        e.preventDefault();
        onConfirm(selected);
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onConfirm, pending, selected]);

  const downOnBackdropRef = useRef(false);

  function toggleExpand(path: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  const subtitle = sources.length === 1
    ? (() => {
        const raw = nameOf(sources[0]!);
        const decorated = localizedNoun(raw, '을/를', i18n.language);
        return t('folder_picker.body_single', { name: `"${decorated}"` });
      })()
    : t('folder_picker.body_multi', { count: sources.length });

  return (
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
        <button type="button" className="cw-close" onClick={onClose} disabled={pending} aria-label={tCommon('actions.close')}>
          <Icon name="x" />
        </button>
        <h2 style={{ margin: '0 0 6px', fontSize: 18, letterSpacing: '-0.015em' }}>{title}</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 12px', fontSize: 13, lineHeight: 1.55 }}>
          {subtitle}
        </p>
        <div className="cw-folder-picker-tree">
          <div
            className={`cw-tree-row${selected === '' ? ' is-selected' : ''}`}
            style={{ paddingLeft: 8 }}
          >
            <span className="cw-tree-chevron" style={{ visibility: 'hidden' }}>
              <Icon name="chevron-right" size={12} />
            </span>
            <button type="button" className="cw-tree-label" onClick={() => setSelected('')}>
              <Icon name="folder" size={14} />
              <span>{t('folder_picker.root')}</span>
            </button>
          </div>
          {tree.map((node) => (
            <PickerNode
              key={node.path}
              node={node}
              depth={1}
              expanded={expanded}
              selected={selected}
              disabledPaths={disabledPaths}
              onToggle={toggleExpand}
              onSelect={setSelected}
            />
          ))}
        </div>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end', marginTop: 18 }}>
          <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>
            {tCommon('actions.cancel')}
          </button>
          <button
            type="button"
            className="cw-btn-primary"
            disabled={pending || selected === null}
            onClick={() => { if (selected !== null) onConfirm(selected); }}
          >
            {pending ? t('folder_picker.submitting') : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
