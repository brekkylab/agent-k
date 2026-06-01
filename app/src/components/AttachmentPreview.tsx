import { useEffect, useRef, useState } from 'react';
import { downloadFileByGlobalPath, parseGlobalPath, type DirentScope } from '@/api/dirents';
import { Icon } from './Icon';
import { FileTypeIcon } from './FileTypeIcon';

interface Props {
  globalPath: string;
  onCopyToShared?: (scope: DirentScope, paths: string[]) => void;
}

export function AttachmentPreview({ globalPath, onCopyToShared }: Props) {
  const filename = globalPath.split('/').pop() ?? globalPath;
  const [menuOpen, setMenuOpen] = useState(false);
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
      <span>{filename}</span>
      {menuOpen && (
        <ul
          className="cw-file-dropdown"
          style={{ top: 'calc(100% + 4px)', left: 0 }}
          onClick={(e) => { e.stopPropagation(); setMenuOpen(false); }}
        >
          <li>
            <button type="button" onClick={handleDownload}>
              <Icon name="download" size={13} /> 다운로드
            </button>
          </li>
          {canCopy && (
            <li>
              <button type="button" onClick={handleCopyToShared}>
                <Icon name="file" size={13} /> 공유 디렉토리로 복사
              </button>
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
