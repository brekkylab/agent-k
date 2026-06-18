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

interface Props {
  projectId: string;
  /** The first column — the shared root, or any folder to start from. */
  rootFolderPath: string;
  /** Label for the first column's header (e.g. the project name; the root's basename otherwise). */
  rootLabel?: string;
  /** Attach files/folders picked from a row's "+". */
  onImport: (items: SessionImportItem[]) => void;
  /** Global paths already staged — file rows show a check instead of the "+". */
  addedPaths: Set<string>;
}

/**
 * macOS Finder-style column (miller) view for the picker's right pane. Each
 * folder opened adds a new column to the right; clicking a file collapses any
 * deeper columns and shows its preview as the trailing pane. The strip scrolls
 * horizontally as the chain grows. Rows carry the same "+" attach affordance and
 * added-check as the left browser. Reuses the browser's cached shared-dirents
 * query, so no extra fetch.
 */
export function FolderColumnsPane({ projectId, rootFolderPath, rootLabel, onImport, addedPaths }: Props) {
  const scope: DirentScope = { kind: 'shared', projectId };
  const { data: entries = [] } = useQuery({
    queryKey: ['dirents', 'shared', projectId],
    queryFn: () => listDirentsRaw(scope, true),
    enabled: Boolean(projectId),
  });

  // Chain of folder paths, one per column (first = the activated folder).
  const [trail, setTrail] = useState<string[]>([rootFolderPath]);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  useEffect(() => { setTrail([rootFolderPath]); setSelectedFile(null); }, [rootFolderPath]);

  // Reveal the newest column / preview as the chain grows.
  const stripRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = stripRef.current;
    if (el) el.scrollLeft = el.scrollWidth;
  }, [trail, selectedFile]);

  const openFolderAt = (colIndex: number, folderPath: string) => {
    setSelectedFile(null);
    setTrail((prev) => [...prev.slice(0, colIndex + 1), folderPath]);
  };
  const selectFileAt = (colIndex: number, filePath: string) => {
    setTrail((prev) => prev.slice(0, colIndex + 1)); // collapse columns to the right
    setSelectedFile(filePath);
  };

  return (
    <div className="cw-folder-columns" ref={stripRef}>
      {trail.map((folderPath, i) => (
        <FolderColumn
          key={`${i}-${folderPath}`}
          entries={entries}
          folderPath={folderPath}
          headerLabel={i === 0 ? rootLabel : undefined}
          // The opened sub-folder (next column) or, on the last column, the picked file.
          selectedChild={trail[i + 1] ?? (i === trail.length - 1 ? selectedFile : null)}
          addedPaths={addedPaths}
          onOpenFolder={(p) => openFolderAt(i, p)}
          onSelectFile={(p) => selectFileAt(i, p)}
          onImport={onImport}
        />
      ))}
      {selectedFile && (
        <div className="cw-folder-columns-preview">
          <FilePreviewPane
            globalPath={selectedFile}
            emptyHint=""
            added={addedPaths.has(selectedFile)}
            onAttach={() => onImport([{ globalPath: selectedFile, filename: selectedFile.split('/').filter(Boolean).pop() ?? selectedFile }])}
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
  selectedChild: string | null;
  addedPaths: Set<string>;
  onOpenFolder: (globalPath: string) => void;
  onSelectFile: (globalPath: string) => void;
  onImport: (items: SessionImportItem[]) => void;
}

function FolderColumn({ entries, folderPath, headerLabel, selectedChild, addedPaths, onOpenFolder, onSelectFile, onImport }: ColumnProps) {
  const { t } = useTranslation('session');
  const { folders, files } = useMemo(
    () => listDirectChildren(entries, folderPath.split('/')),
    [entries, folderPath],
  );
  const total = folders.length + files.length;
  const name = headerLabel ?? folderPath.split('/').filter(Boolean).pop() ?? folderPath;

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
                className={`cw-folder-list-row${selectedChild === d.path ? ' is-active' : ''}`}
                onClick={() => onOpenFolder(d.path)}
              >
                <Icon name="folder" size={16} />
                <span className="cw-folder-list-name">{nameOf(d)}</span>
                {/* + attaches the folder's files (matches the left browser). */}
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
            {files.map((f) => (
              <div
                key={f.path}
                className={`cw-folder-list-row${selectedChild === f.path ? ' is-active' : ''}${addedPaths.has(f.path) ? ' is-added' : ''}`}
                onClick={() => onSelectFile(f.path)}
              >
                <FileTypeIcon filename={nameOf(f)} size={16} />
                <span className="cw-folder-list-name">{nameOf(f)}</span>
                {addedPaths.has(f.path) ? (
                  <span className="cw-folder-list-check" aria-label={t('shared_files.added')} title={t('shared_files.added')}>
                    <Icon name="check" size={14} />
                  </span>
                ) : (
                  <button
                    type="button"
                    className="cw-folder-list-add"
                    aria-label={t('shared_files.import')}
                    title={t('shared_files.import')}
                    onClick={(e) => { e.stopPropagation(); onImport([{ globalPath: f.path, filename: nameOf(f) }]); }}
                  >
                    <Icon name="plus" size={13} />
                  </button>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
