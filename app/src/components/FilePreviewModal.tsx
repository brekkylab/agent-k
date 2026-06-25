import { useEffect, useId, useRef } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useDialogEscape } from '@/lib/useDialogEscape';
import { Icon } from './Icon';
import { useFilePreview } from './preview/useFilePreview';
import { PreviewBody } from './preview/PreviewBody';

interface Props {
  globalPath: string;
  onClose: () => void;
}

export function FilePreviewModal({ globalPath, onClose }: Props) {
  const { t } = useTranslation('common');
  const { state, filename, download, isStage } = useFilePreview(globalPath);

  useDialogEscape(onClose);

  // Initial focus + restore on close (mirrors SessionsOverlay/WebhookTokenDialog),
  // a minimal Tab focus trap, and background scroll lock for the large modal.
  const modalRef = useRef<HTMLDivElement>(null);
  const closeBtnRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const prevActive = document.activeElement as HTMLElement | null;
    closeBtnRef.current?.focus();
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.body.style.overflow = prevOverflow;
      prevActive?.focus?.();
    };
  }, []);

  function onTrapKeyDown(e: React.KeyboardEvent) {
    if (e.key !== 'Tab') return;
    const root = modalRef.current;
    if (!root) return;
    // Deliberately excludes `iframe`: focusing the sandboxed HTML-preview frame
    // would swallow keydowns (ESC/Tab) inside its browsing context, breaking the
    // dialog's keyboard handling. The frame stays scrollable via mouse.
    const focusables = root.querySelectorAll<HTMLElement>(
      'button, [href], input, [tabindex]:not([tabindex="-1"])',
    );
    if (focusables.length === 0) return;
    const first = focusables[0]!;
    const last = focusables[focusables.length - 1]!;
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  }

  // Click anywhere that isn't the content/chrome/zoom-pill (i.e. the dimmed
  // area) dismisses — works for both the media stage's margins and the sheet's
  // surrounding dim, regardless of nesting depth.
  function maybeClose(e: React.MouseEvent) {
    if (!(e.target as HTMLElement).closest('.cw-preview-content, .cw-preview-chrome, .cw-zoom-controls')) {
      onClose();
    }
  }

  const titleId = useId();
  return createPortal(
    <div
      className="cw-preview-backdrop"
      data-mode={isStage ? 'stage' : 'sheet'}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      ref={modalRef}
      onKeyDown={onTrapKeyDown}
      onMouseDown={maybeClose}
      // Stop clicks bubbling through the React portal tree to an ancestor (e.g.
      // the AttachmentPreview chip's onClick toggle) when mounted under one.
      onClick={(e) => e.stopPropagation()}
    >
      <div className="cw-preview-chrome">
        <span id={titleId} className="cw-preview-title" title={filename}>{filename}</span>
        <div className="cw-preview-actions">
          <button type="button" aria-label={t('preview.download')} onClick={download}>
            <Icon name="download" size={16} />
          </button>
          <button type="button" ref={closeBtnRef} aria-label={t('preview.close')} onClick={onClose}>
            <Icon name="x" size={18} />
          </button>
        </div>
      </div>

      <PreviewBody state={state} filename={filename} onDownload={download} />
    </div>,
    document.body,
  );
}
