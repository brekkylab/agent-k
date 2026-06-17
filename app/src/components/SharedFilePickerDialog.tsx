// 16:9 two-pane picker for attaching project shared files to the home composer's
// first message. Left: SharedFilesBrowser (reused from the session). Right: a live
// preview of the highlighted file (FilePreviewPane). The browser's per-row "+"
// adds a file via onImport; the dialog stays open so several can be added.

import { useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { SharedFilesBrowser, type SessionImportItem } from './SharedFilesPanel';
import { FilePreviewPane } from './preview/FilePreviewPane';
import { AttachmentChip } from './AttachmentChip';
import { useDialogEscape } from '@/lib/useDialogEscape';
import { Icon } from './Icon';

interface Props {
  projectId: string;
  projectName?: string;
  /** Add the picked shared files to the composer (per-row "+" in the browser). */
  onImport: (items: SessionImportItem[]) => void;
  /** Files already staged on the composer — shown as a tray so the user sees what's picked. */
  staged: SessionImportItem[];
  /** Remove a staged file (by global path) from within the dialog. */
  onRemove: (globalPath: string) => void;
  onClose: () => void;
}

export function SharedFilePickerDialog({ projectId, projectName, onImport, staged, onRemove, onClose }: Props) {
  const { t } = useTranslation('project');
  const { t: tCommon } = useTranslation('common');
  const [activePath, setActivePath] = useState<string | null>(null);
  const downOnBackdropRef = useRef(false);
  const addedPaths = useMemo(() => new Set(staged.map((s) => s.globalPath)), [staged]);

  useDialogEscape(onClose);

  return (
    <div
      className="cw-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      onMouseDown={(e) => { downOnBackdropRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
        const wasDown = downOnBackdropRef.current;
        downOnBackdropRef.current = false;
        if (wasDown && e.target === e.currentTarget) onClose();
      }}
    >
      <div className="cw-dialog cw-shared-picker">
        <button type="button" className="cw-close" onClick={onClose} aria-label={tCommon('actions.close')}>
          <Icon name="x" />
        </button>
        <h2 className="cw-shared-picker-title">{t('home.shared_picker.title')}</h2>
        <p className="cw-shared-picker-sub">{t('home.shared_picker.subtitle')}</p>
        <div className="cw-shared-picker-body">
          <div className="cw-shared-picker-list">
            <SharedFilesBrowser
              projectId={projectId}
              projectName={projectName}
              onImport={onImport}
              onActivate={setActivePath}
              activePath={activePath}
              addedPaths={addedPaths}
            />
          </div>
          <div className="cw-shared-picker-preview">
            <FilePreviewPane globalPath={activePath} emptyHint={t('home.shared_picker.preview_empty')} />
          </div>
        </div>
        <div className="cw-shared-picker-foot">
          {/* Staged files live behind the dialog on the composer, so mirror them here
              as a removable tray — the count makes the attached state unmistakable. */}
          <div className="cw-shared-picker-staged">
            {staged.length > 0 ? (
              <>
                <span className="cw-shared-picker-count">{t('home.shared_picker.staged_count', { count: staged.length })}</span>
                {staged.map((item) => (
                  <AttachmentChip
                    key={item.globalPath}
                    filename={item.filename}
                    status="uploaded"
                    shared
                    onRemove={() => onRemove(item.globalPath)}
                  />
                ))}
              </>
            ) : (
              <span className="cw-shared-picker-count is-empty">{t('home.shared_picker.staged_none')}</span>
            )}
          </div>
          <button type="button" className="cw-btn-primary" onClick={onClose}>
            {tCommon('actions.done')}
          </button>
        </div>
      </div>
    </div>
  );
}
