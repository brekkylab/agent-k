import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { downloadFileByGlobalPath, parseGlobalPath, type DirentScope } from '@/api/dirents';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';
import { FilePreviewModal } from './FilePreviewModal';

interface Props {
  globalPath: string;
  onCopyToShared?: (scope: DirentScope, paths: string[]) => void;
}

export function AttachmentPreview({ globalPath, onCopyToShared }: Props) {
  const { t } = useTranslation('session');
  const filename = globalPath.split('/').pop() ?? globalPath;
  const [menuOpen, setMenuOpen] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const chipRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    function onPtr(e: PointerEvent) {
      if (chipRef.current && !chipRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    }
    document.addEventListener('pointerdown', onPtr);
    return () => document.removeEventListener('pointerdown', onPtr);
  }, [menuOpen]);

  const parsed = parseGlobalPath(globalPath);
  const canCopy = Boolean(onCopyToShared && parsed);

  function handleDownload() {
    void downloadFileByGlobalPath(globalPath);
    setMenuOpen(false);
  }

  function handleCopyToShared() {
    if (onCopyToShared && parsed) {
      onCopyToShared(parsed.scope, [parsed.relativePath]);
    }
    setMenuOpen(false);
  }

  return (
    <div
      ref={chipRef}
      className="cw-attach-chip cw-attach-chip--file"
      style={{ cursor: 'pointer', position: 'relative', overflow: 'visible' }}
      onClick={() => setMenuOpen((prev) => !prev)}
      title={filename}
    >
      <FileTypeIcon filename={filename} size={16} />
      <span className="cw-attach-name">{filename}</span>
      {menuOpen && (
        <ul
          className="cw-file-dropdown"
          style={{ top: 'calc(100% + 4px)', left: 0 }}
          onClick={(e) => { e.stopPropagation(); setMenuOpen(false); }}
        >
          <li>
            <button type="button" onClick={() => { setMenuOpen(false); setPreviewing(true); }}>
              <Icon name="eye" size={13} /> {t('artifact.preview')}
            </button>
          </li>
          <li>
            <button type="button" onClick={handleDownload}>
              <Icon name="download" size={13} /> {t('artifact.download')}
            </button>
          </li>
          {canCopy && (
            <li>
              <button type="button" onClick={handleCopyToShared}>
                <Icon name="file" size={13} /> {t('artifact.copy_to_shared')}
              </button>
            </li>
          )}
        </ul>
      )}
      {previewing && (
        <FilePreviewModal globalPath={globalPath} onClose={() => setPreviewing(false)} />
      )}
    </div>
  );
}
