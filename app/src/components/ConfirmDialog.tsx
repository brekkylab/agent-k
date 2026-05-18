// Reusable confirm dialog using Cowork DS .cw-dialog primitives.
// Destructive variant tints the confirm button using the brick destructive token.

import { useEffect, useRef } from 'react';
import { Icon } from './Icon';

interface ConfirmDialogProps {
  title: string;
  body: string;
  confirmLabel: string;
  cancelLabel?: string;
  destructive?: boolean;
  pending?: boolean;
  onConfirm: () => void;
  onClose: () => void;
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
}: ConfirmDialogProps) {
  // Close on Escape — small UX detail that reads as polish.
  useEffect(() => {
    function onKey(e: KeyboardEvent) { if (e.key === 'Escape' && !pending) onClose(); }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, pending]);

  // Track where mousedown started so a drag (e.g. selecting body text) that
  // ends on the backdrop doesn't close the dialog.
  const downOnBackdropRef = useRef(false);

  return (
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
          <button type="button" className="cw-btn-secondary" onClick={onClose} disabled={pending}>{cancelLabel}</button>
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
            {pending ? '처리 중…' : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
