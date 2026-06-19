import { useEffect, useMemo, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { listDirentsRaw, type DirentScope } from '@/api/dirents';
import { listDirectChildren, expandDirentPaths, nameOf } from '@/domain/files';
import type { BackendDirent } from '@/api/backend-types';
import type { SessionImportItem } from '../SharedFilesPanel';
import { FilePreviewPane } from './FilePreviewPane';
import { FileTypeIcon } from '../FileTypeIcon';
import { Icon } from '../Icon';

const basename = (p: string) => p.split('/').filter(Boolean).pop() ?? p;

interface Props {
  projectId: string;
  /** The first column — the shared root, or any folder to start from. */
  rootFolderPath: string;
  /** Label for the first column's header (e.g. the project name; the root's basename otherwise). */
  rootLabel?: string;
  /** Attach files/folders (a checkbox tick, a shift-range, or a folder's "+"). */
  onImport: (items: SessionImportItem[]) => void;
  /** Un-stage a file by global path (un-ticking its checkbox). */
  onRemove: (globalPath: string) => void;
  /** Global paths already staged — a file's checkbox is ticked. */
  addedPaths: Set<string>;
}

/**
 * macOS Finder-style column (miller) view for the picker's right pane. Each folder
 * opened adds a column to the right; clicking a file's name previews it in the
 * trailing pane. Attaching is a per-file checkbox (= staged state), so selecting
 * IS attaching — no separate "add" step. Shift-clicking a checkbox ticks the whole
 * range from the last one. Reuses the browser's cached shared-dirents query.
 */
export function FolderColumnsPane({ projectId, rootFolderPath, rootLabel, onImport, onRemove, addedPaths }: Props) {
  const scope: DirentScope = { kind: 'shared', projectId };
  const { data: entries = [] } = useQuery({
    queryKey: ['dirents', 'shared', projectId],
    queryFn: () => listDirentsRaw(scope, true),
    enabled: Boolean(projectId),
  });

  // Chain of folder paths, one per column (first = the activated folder).
  const [trail, setTrail] = useState<string[]>([rootFolderPath]);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  // Last checkbox ticked, for shift-range (scoped to its column).
  const anchorRef = useRef<{ col: number; path: string } | null>(null);
  useEffect(() => { setTrail([rootFolderPath]); setSelectedFile(null); anchorRef.current = null; }, [rootFolderPath]);

  // Reveal the newest column / preview as the chain grows.
  const stripRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = stripRef.current;
    if (el) el.scrollLeft = el.scrollWidth;
  }, [trail, selectedFile]);

  const fileOrderAt = (col: number) =>
    listDirectChildren(entries, trail[col].split('/')).files.map((f) => f.path);

  const openFolderAt = (col: number, folderPath: string) => {
    setSelectedFile(null);
    anchorRef.current = null;
    setTrail((prev) => [...prev.slice(0, col + 1), folderPath]);
  };

  // Preview a file (name click) — show it as the trailing pane, collapsing any
  // deeper columns. Does NOT change what's attached, but DOES set the shift-range
  // anchor (Finder-style: a plain click is the point a later shift-click extends
  // from), so click file → shift-click another ticks the whole range.
  const previewFileAt = (col: number, path: string) => {
    setTrail((prev) => prev.slice(0, col + 1));
    setSelectedFile(path);
    anchorRef.current = { col, path };
  };

  // Tick/un-tick a file's checkbox (= attach/un-stage). Shift ticks the range from
  // the last-ticked checkbox in the same column. Never touches preview/columns.
  const toggleFileAt = (col: number, e: React.MouseEvent, path: string) => {
    if (e.shiftKey && anchorRef.current && anchorRef.current.col === col) {
      const order = fileOrderAt(col);
      const a = order.indexOf(anchorRef.current.path);
      const b = order.indexOf(path);
      if (a !== -1 && b !== -1) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        onImport(order.slice(lo, hi + 1).map((p) => ({ globalPath: p, filename: basename(p) })));
        return;
      }
    }
    if (addedPaths.has(path)) onRemove(path);
    else onImport([{ globalPath: path, filename: basename(path) }]);
    anchorRef.current = { col, path };
  };

  return (
    <div className="cw-folder-columns" ref={stripRef}>
      {trail.map((folderPath, i) => (
        <FolderColumn
          key={`${i}-${folderPath}`}
          entries={entries}
          folderPath={folderPath}
          headerLabel={i === 0 ? rootLabel : undefined}
          openedFolder={trail[i + 1] ?? null}
          previewedFile={selectedFile}
          addedPaths={addedPaths}
          onOpenFolder={(p) => openFolderAt(i, p)}
          onPreviewFile={(p) => previewFileAt(i, p)}
          onToggleFile={(e, p) => toggleFileAt(i, e, p)}
          onImport={onImport}
        />
      ))}
      {selectedFile && (
        <div className="cw-folder-columns-preview">
          <FilePreviewPane
            globalPath={selectedFile}
            emptyHint=""
            added={addedPaths.has(selectedFile)}
            onAttach={() => onImport([{ globalPath: selectedFile, filename: basename(selectedFile) }])}
            onRemove={() => onRemove(selectedFile)}
          />
        </div>
      )}
    </div>
  );
}

interface ColumnProps {
  entries: BackendDirent[];
  folderPath: string;
  headerLabel?: string;
  /** The sub-folder opened from this column (highlighted), if any. */
  openedFolder: string | null;
  /** The file currently previewed (highlighted), if it lives in this column. */
  previewedFile: string | null;
  addedPaths: Set<string>;
  onOpenFolder: (globalPath: string) => void;
  onPreviewFile: (globalPath: string) => void;
  onToggleFile: (e: React.MouseEvent, globalPath: string) => void;
  onImport: (items: SessionImportItem[]) => void;
}

function FolderColumn({ entries, folderPath, headerLabel, openedFolder, previewedFile, addedPaths, onOpenFolder, onPreviewFile, onToggleFile, onImport }: ColumnProps) {
  const { t } = useTranslation('session');
  const { folders, files } = useMemo(
    () => listDirectChildren(entries, folderPath.split('/')),
    [entries, folderPath],
  );
  const total = folders.length + files.length;
  const name = headerLabel ?? basename(folderPath);

  return (
    <div className="cw-folder-column">
      <div className="cw-folder-column-head">
        <Icon name="folder" size={14} />
        <span className="cw-folder-column-name" title={name}>{name}</span>
      </div>
      <div className="cw-folder-column-body">
        {total === 0 ? (
          <p className="cw-folder-column-empty">{t('shared_files.empty_title')}</p>
        ) : (
          <div className="cw-folder-list">
            {folders.map((d) => (
              <div
                key={d.path}
                className={`cw-folder-list-row${openedFolder === d.path ? ' is-active' : ''}`}
                onClick={() => onOpenFolder(d.path)}
              >
                <span className="cw-folder-chevron" aria-hidden="true"><Icon name="chevron-right" size={13} /></span>
                <Icon name="folder" size={16} />
                <span className="cw-folder-list-name">{nameOf(d)}</span>
                {/* + attaches every file in the folder (matches the left browser). */}
                <button
                  type="button"
                  className="cw-folder-list-add"
                  aria-label={t('shared_files.import')}
                  title={t('shared_files.import')}
                  onClick={(e) => { e.stopPropagation(); onImport(expandDirentPaths(entries, [d.path])); }}
                >
                  <Icon name="plus" size={13} />
                </button>
              </div>
            ))}
            {files.map((f) => {
              const checked = addedPaths.has(f.path);
              return (
                <div
                  key={f.path}
                  className={`cw-folder-list-row${previewedFile === f.path ? ' is-active' : ''}`}
                  // Shift-click range-ticks; Cmd (mac) / Ctrl (win/linux) toggles just this
                  // one into the selection — both anywhere on the row, not only the checkbox.
                  // A plain click previews.
                  onClick={(e) => (e.shiftKey || e.metaKey || e.ctrlKey ? onToggleFile(e, f.path) : onPreviewFile(f.path))}
                >
                  {/* Checkbox = attached state: ticking it stages the file (shift = range). */}
                  <button
                    type="button"
                    role="checkbox"
                    aria-checked={checked}
                    className={`cw-folder-check${checked ? ' is-checked' : ''}`}
                    aria-label={checked ? t('shared_files.remove') : t('shared_files.import')}
                    title={checked ? t('shared_files.remove') : t('shared_files.import')}
                    onClick={(e) => { e.stopPropagation(); onToggleFile(e, f.path); }}
                  >
                    {checked && <Icon name="check" size={12} />}
                  </button>
                  <FileTypeIcon filename={nameOf(f)} size={16} />
                  <span className="cw-folder-list-name">{nameOf(f)}</span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
