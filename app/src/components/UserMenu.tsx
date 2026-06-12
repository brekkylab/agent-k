// ⋯ menu on the sidebar footer's user row. It opens upward since it sits at the
// bottom of the screen. The popover is rendered into a body portal so it isn't
// clipped by the sidebar's overflow. (Follows SessionCardMenu, but positioned
// from the bottom edge rather than the top.)
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { useDialogEscape } from '@/lib/useDialogEscape';

interface UserMenuProps {
  onOpenSettings: () => void;
  onLogout: () => void;
}
interface MenuRect { bottom: number; left: number; }

const ITEM_BASE: CSSProperties = {
  display: 'flex', alignItems: 'center', gap: 8, width: '100%', textAlign: 'left',
  padding: '8px 10px', border: 0, background: 'transparent', fontSize: 12.5,
  borderRadius: 'var(--cw-radius-sm)', cursor: 'pointer',
};

export function UserMenu({ onOpenSettings, onLogout }: UserMenuProps) {
  const { t } = useTranslation('common');
  const [open, setOpen] = useState(false);
  const [rect, setRect] = useState<MenuRect | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  // Position from the bottom edge so it opens upward, 6px above the trigger.
  useLayoutEffect(() => {
    if (!open || !buttonRef.current) return;
    function place() {
      const r = buttonRef.current!.getBoundingClientRect();
      const WIDTH = 180;
      const left = Math.max(8, Math.min(window.innerWidth - WIDTH - 8, r.left));
      const bottom = window.innerHeight - r.top + 6;
      setRect({ bottom, left });
    }
    place();
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
    };
  }, [open]);

  useDialogEscape(() => setOpen(false), { disabled: !open });
  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      const target = e.target as Node;
      if (buttonRef.current?.contains(target) || popoverRef.current?.contains(target)) return;
      setOpen(false);
    }
    document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, [open]);

  function hover(on: boolean) {
    return (e: { currentTarget: HTMLButtonElement }) => {
      e.currentTarget.style.background = on ? 'var(--cw-paper-3)' : 'transparent';
    };
  }

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className="cw-icon-button"
        aria-label={t('actions.user_menu')}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <Icon name="more" />
      </button>
      {open && rect && createPortal(
        <div
          ref={popoverRef}
          role="menu"
          style={{
            position: 'fixed', bottom: rect.bottom, left: rect.left, minWidth: 180,
            background: 'var(--cw-paper)', border: '1px solid var(--cw-line)',
            borderRadius: 'var(--cw-radius-md)', boxShadow: 'var(--cw-shadow-popover)',
            padding: 4, zIndex: 100,
          }}
        >
          <button
            type="button" role="menuitem"
            onClick={() => { setOpen(false); onOpenSettings(); }}
            style={{ ...ITEM_BASE, color: 'var(--cw-ink)' }}
            onMouseEnter={hover(true)} onMouseLeave={hover(false)}
          >
            <Icon name="settings" size={14} /> {t('actions.user_settings')}
          </button>
          <button
            type="button" role="menuitem"
            onClick={() => { setOpen(false); onLogout(); }}
            style={{ ...ITEM_BASE, color: 'var(--cw-destructive)' }}
            onMouseEnter={hover(true)} onMouseLeave={hover(false)}
          >
            <Icon name="log-out" size={14} /> {t('actions.logout')}
          </button>
        </div>,
        document.body,
      )}
    </>
  );
}
