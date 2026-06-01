import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon, type IconName } from './Icon';
import { shareMeta } from '../domain/metadata';
import type { ShareMode, User } from '../domain/types';

const SHARE_MODES = Object.keys(shareMeta) as ShareMode[];

export function EmptyState({
  title,
  body,
  action,
  onAction,
  chip = 'AI',
}: {
  title: string;
  body: string;
  action?: string;
  onAction?: () => void;
  // Override the chip label. Defaults to 'AI' because the original use site was
  // the chat surface. For non-chat empty states pass a context-appropriate chip
  // (e.g. '+', '🗂', or a ReactNode).
  chip?: ReactNode;
}) {
  return (
    <div className="cw-empty-state">
      <span className="cw-empty-chip">{chip}</span>
      <div>
        <b>{title}</b>
        <p>{body}</p>
        {action && onAction && <button className="cw-btn-secondary" onClick={onAction}>{action}</button>}
      </div>
    </div>
  );
}

export function SectionLabel({ children }: { children: ReactNode }) {
  return <div className="cw-section-label-app">{children}</div>;
}

export function Avatar({ user, small = false }: { user: User; small?: boolean }) {
  return <span className={`cw-avatar-app ${small ? 'small' : ''}`} style={{ background: user.color }}>{user.avatar}</span>;
}

export function AvatarStack({ users }: { users: User[] }) {
  return <span className="cw-avatar-stack">{users.slice(0, 4).map((user) => <Avatar user={user} small key={user.id} />)}</span>;
}

export function IconPocket({ tone, icon, compact = false }: { tone: string; icon: IconName; compact?: boolean }) {
  return <span className={`cw-pocket cw-nav-${tone} ${compact ? 'is-compact' : ''}`.trim()}><Icon name={icon} size={compact ? 12 : 13} /></span>;
}

export function byId<T extends { id: string }>(items: T[], id: string): T {
  const item = items.find((candidate) => candidate.id === id);
  if (!item) throw new Error(`Missing item: ${id}`);
  return item;
}

export function InfoRow({ icon, title, meta, children }: { icon: IconName; title: string; meta: string; children: ReactNode }) { return <article className="cw-info-row"><IconPocket tone="neutral" icon={icon} /><div><b>{title}</b><p>{children}</p></div><span>{meta}</span></article>; }

export function ActivityRow({ title, date, children }: { title: string; date: string; children: ReactNode }) { return <article className="cw-activity-row"><span><Icon name="recap" /></span><div><b>{title}</b><p>{children}</p></div><time>{date}</time></article>; }

export function SharePill({ mode, compact = false }: { mode: ShareMode; compact?: boolean }) {
  const { t } = useTranslation('common');
  const labelKey = compact ? `share.${mode}.short_label` : `share.${mode}.label`;
  return (
    <span className={`cw-share-pill ${shareMeta[mode].className}`}>
      <Icon name={shareMeta[mode].icon} size={compact ? 11 : 12} />
      {t(labelKey)}
    </span>
  );
}

export function ShareSelect({ mode, onChange }: { mode: ShareMode; onChange: (mode: ShareMode) => void }) {
  const { t } = useTranslation('common');
  const [open, setOpen] = useState(false);
  const [focusIdx, setFocusIdx] = useState(() => SHARE_MODES.indexOf(mode));
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const measureRef = useRef<HTMLSpanElement>(null);
  const [width, setWidth] = useState<number | null>(null);

  // Re-measure when the visible label or its language changes. The hidden
  // .cw-share-select-measure clone shares the trigger's box styling, so its
  // intrinsic width is the value the real trigger should transition to.
  useLayoutEffect(() => {
    if (measureRef.current) {
      setWidth(Math.ceil(measureRef.current.getBoundingClientRect().width));
    }
  }, [mode, t]);

  // Outside-click dismissal, only mounted while open. Keyboard is handled on
  // the trigger's onKeyDown (see below) — the trigger keeps DOM focus while
  // the panel is open, so a document keydown listener isn't needed and would
  // re-read the very keypress that opened the panel (it's attached by this
  // effect mid-event), closing it again on Enter.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (!wrapRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open]);

  function commit(next: ShareMode) {
    onChange(next);
    setOpen(false);
    triggerRef.current?.focus();
  }

  function toggle() {
    setOpen((wasOpen) => {
      if (!wasOpen) setFocusIdx(SHARE_MODES.indexOf(mode));
      return !wasOpen;
    });
  }

  function onTriggerKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (!open) {
      // Closed: open on Enter / Space / ArrowDown, focusing the current mode.
      if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        setFocusIdx(SHARE_MODES.indexOf(mode));
        setOpen(true);
      }
      return;
    }
    // Open: navigate / commit / dismiss. Handled here rather than on a
    // document listener so the keydown that opened the panel can't be
    // re-read and immediately close it.
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        setFocusIdx((i) => (i + 1) % SHARE_MODES.length);
        break;
      case 'ArrowUp':
        event.preventDefault();
        setFocusIdx((i) => (i - 1 + SHARE_MODES.length) % SHARE_MODES.length);
        break;
      case 'Enter':
      case ' ':
        event.preventDefault();
        commit(SHARE_MODES[focusIdx]);
        break;
      case 'Escape':
        event.preventDefault();
        setOpen(false);
        break;
      case 'Tab':
        // Let focus move naturally, but don't leave an orphaned open panel.
        setOpen(false);
        break;
    }
  }

  return (
    <div ref={wrapRef} className="cw-share-select-wrap">
      <button
        ref={triggerRef}
        type="button"
        className={`cw-share-select ${shareMeta[mode].className}`}
        style={width != null ? { width } : undefined}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={toggle}
        onKeyDown={onTriggerKeyDown}
      >
        <Icon name={shareMeta[mode].icon} />
        <span>{t(`share.${mode}.label`)}</span>
        <span className="cw-share-select-caret" aria-hidden="true" />
      </button>
      <span ref={measureRef} className="cw-share-select cw-share-select-measure" aria-hidden="true">
        <Icon name={shareMeta[mode].icon} />
        <span>{t(`share.${mode}.label`)}</span>
        <span className="cw-share-select-caret" aria-hidden="true" />
      </span>
      {open && (
        <ul role="listbox" className="cw-share-select-panel">
          {SHARE_MODES.map((key, idx) => (
            <li
              key={key}
              role="option"
              aria-selected={key === mode}
              className={
                'cw-share-select-option' +
                (key === mode ? ' is-selected' : '') +
                (idx === focusIdx ? ' is-focused' : '')
              }
              onMouseEnter={() => setFocusIdx(idx)}
              onClick={() => commit(key)}
            >
              <Icon name={shareMeta[key].icon} />
              <span>{t(`share.${key}.label`)}</span>
              {key === mode && <span className="cw-share-select-check" aria-hidden="true">✓</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/// Ghost icon-only button. Base style ships in `.cw-icon-button` (globals.css);
/// pass `className` to layer modifier classes (e.g. `cw-rail-action` adds
/// row-hover gating and smaller sizing). `label` is the aria-label (required
/// for accessibility); `title` overrides the hover tooltip when it should
/// differ from the label.
/// `stopPropagation` is for buttons sitting inside a clickable parent row.
/// Pass `expandedText` to turn the button into a 30×30 square that grows on
/// hover to reveal that text — opt-in is implicit (presence of the prop).
/// `confirmText` makes the button two-click: first click arms (and shows
/// this text), second click fires `onClick`. Mousing off the armed button
/// disarms it.
export function IconButton({
  icon,
  label,
  title,
  onClick,
  disabled = false,
  className,
  iconSize = 15,
  stopPropagation = false,
  expandedText,
  confirmText,
}: {
  icon: IconName;
  label: string;
  title?: string;
  onClick: () => void;
  disabled?: boolean;
  className?: string;
  iconSize?: number;
  stopPropagation?: boolean;
  expandedText?: string;
  confirmText?: string;
}) {
  const [armed, setArmed] = useState(false);
  const useConfirm = Boolean(confirmText) && !disabled;
  // Expandable layout when either the consumer opted in (via expandedText) or
  // the button is currently armed (so the confirmation text has room to show).
  const expandable = Boolean(expandedText) || armed;
  const visibleText = armed ? confirmText : (expandedText ?? label);
  const cls = `cw-icon-button${expandable ? ' is-expandable' : ''}${armed ? ' is-armed' : ''}${className ? ` ${className}` : ''}`;
  return (
    <button
      type="button"
      aria-label={armed && confirmText ? confirmText : label}
      title={armed && confirmText ? confirmText : (title ?? label)}
      onClick={(e) => {
        if (stopPropagation) e.stopPropagation();
        if (disabled) return;
        if (useConfirm && !armed) {
          setArmed(true);
          return;
        }
        setArmed(false);
        onClick();
      }}
      onMouseLeave={armed ? () => setArmed(false) : undefined}
      disabled={disabled}
      className={cls}
    >
      {expandable ? (
        <>
          <span className="cw-icon-button-icon"><Icon name={icon} size={iconSize} /></span>
          <span className="cw-icon-button-label">{visibleText}</span>
        </>
      ) : (
        <Icon name={icon} size={iconSize} />
      )}
    </button>
  );
}
