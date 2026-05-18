// ⋯ overflow menu attached to a session card or sidebar row.
// The popover renders into a body-level portal so it isn't clipped by
// scrolling/overflow ancestors (cw-sidebar-app, cw-session-rail, ...).
// Positioning is computed at open time from the ⋯ button's bounding rect.

import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from './Icon';

interface SessionCardMenuProps {
  onDelete: () => void;
}

interface MenuRect { top: number; left: number; }

export function SessionCardMenu({ onDelete }: SessionCardMenuProps) {
  const [open, setOpen] = useState(false);
  const [rect, setRect] = useState<MenuRect | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  // Compute popover position relative to the ⋯ trigger. Right-align so the
  // menu fits whether trigger sits at the edge of a 200px sidebar or in a
  // wider card grid cell. Recompute on resize/scroll while open.
  useLayoutEffect(() => {
    if (!open || !buttonRef.current) return;
    function place() {
      const r = buttonRef.current!.getBoundingClientRect();
      const POPOVER_WIDTH = 150;
      const left = Math.max(8, Math.min(window.innerWidth - POPOVER_WIDTH - 8, r.right - POPOVER_WIDTH));
      const top = r.bottom + 4;
      setRect({ top, left });
    }
    place();
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
    };
  }, [open]);

  // Close on outside click or Escape.
  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      const target = e.target as Node;
      if (buttonRef.current?.contains(target)) return;
      if (popoverRef.current?.contains(target)) return;
      setOpen(false);
    }
    function onKey(e: KeyboardEvent) { if (e.key === 'Escape') setOpen(false); }
    document.addEventListener('mousedown', onDocClick);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDocClick);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        aria-label="세션 옵션"
        aria-expanded={open}
        onClick={(e) => { e.stopPropagation(); setOpen((v) => !v); }}
        style={{
          width: 26,
          height: 26,
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          border: 0,
          background: 'transparent',
          color: 'var(--cw-ink-4)',
          borderRadius: 6,
          padding: 0,
          cursor: 'pointer',
        }}
      >
        <Icon name="more" size={14} />
      </button>
      {open && rect && createPortal(
        <div
          ref={popoverRef}
          role="menu"
          onClick={(e) => e.stopPropagation()}
          style={{
            position: 'fixed',
            top: rect.top,
            left: rect.left,
            minWidth: 150,
            background: 'var(--cw-paper)',
            border: '1px solid var(--cw-line)',
            borderRadius: 'var(--cw-radius-md)',
            boxShadow: 'var(--cw-shadow-popover)',
            padding: 4,
            zIndex: 100,
          }}
        >
          <button
            type="button"
            role="menuitem"
            onClick={(e) => { e.stopPropagation(); setOpen(false); onDelete(); }}
            style={{
              display: 'block',
              width: '100%',
              textAlign: 'left',
              padding: '7px 10px',
              border: 0,
              background: 'transparent',
              color: 'var(--cw-destructive)',
              fontSize: 12.5,
              borderRadius: 'var(--cw-radius-sm)',
              cursor: 'pointer',
            }}
            onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.background = 'var(--cw-paper-3)'; }}
            onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.background = 'transparent'; }}
          >
            세션 삭제
          </button>
        </div>,
        document.body,
      )}
    </>
  );
}
