// Reusable confirm dialog using Cowork DS .cw-dialog primitives.
// Destructive variant tints the confirm button using the brick destructive token.

import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from './Icon';
import { useDialogEscape } from '@/lib/useDialogEscape';

interface ConfirmDialogProps {
  title: string;
  body: string;
  confirmLabel: string;
  cancelLabel?: string;
  destructive?: boolean;
  pending?: boolean;
  onConfirm: () => void;
  onClose: () => void;
  confirmOnEnter?: boolean;
}

export function ConfirmDialog({
  title,
  body,
  confirmLabel,
  cancelLabel = '취소',
  destructive,
  pending,
  onConfirm,
  onClose,
  confirmOnEnter = false,
}: ConfirmDialogProps) {
  // ESC routes through the modal stack so dialogs win over background page
  // handlers (e.g. file selection clear). Enter stays on a separate listener
  // since it isn't part of the modal-stack contract.
  useDialogEscape(onClose, { disabled: pending });
  useEffect(() => {
    if (!confirmOnEnter) return;
    function onKey(e: KeyboardEvent) {
      if (pending) return;
      if (e.key === 'Enter') onConfirm();
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onConfirm, pending, confirmOnEnter]);

  // Track where mousedown started so a drag (e.g. selecting body text) that
  // ends on the backdrop doesn't close the dialog.
  const downOnBackdropRef = useRef(false);

  return createPortal(
    <div
      className="cw-dialog-backdrop"
      role="dialog"
      aria-modal="true"
      onMouseDown={(e) => { downOnBackdropRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
        const wasDownOnBackdrop = downOnBackdropRef.current;
        downOnBackdropRef.current = false;
        if (!wasDownOnBackdrop) return;
        if (e.target === e.currentTarget && !pending) onClose();
      }}
    >
      <div className="cw-dialog">
        <button className="cw-close" onClick={onClose} aria-label="close" disabled={pending}>
          <Icon name="x" />
        </button>
        <h2 style={{ margin: '0 0 8px', fontSize: 18, letterSpacing: '-0.015em' }}>{title}</h2>
        <p style={{ color: 'var(--cw-ink-3)', margin: '0 0 18px', fontSize: 13, lineHeight: 1.6 }}>{body}</p>
        <div style={{ display: 'flex', gap: 10, justifyContent: 'flex-end' }}>
          <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>
            {cancelLabel}
            {confirmOnEnter && <span style={{ opacity: 0.6, fontSize: 11 }}>Esc</span>}
          </button>
          <button
            type="button"
            className="cw-btn-primary"
            onClick={onConfirm}
            disabled={pending}
            style={destructive ? {
              background: 'var(--cw-destructive)',
              borderColor: 'var(--cw-destructive)',
            } : undefined}
          >
            {pending ? '처리 중…' : (
              <>
                {confirmLabel}
                {confirmOnEnter && <Icon name="corner-down-left" size={12} style={{ opacity: 0.7 }} />}
              </>
            )}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
