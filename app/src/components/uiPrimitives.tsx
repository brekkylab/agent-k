import { useState, type ReactNode } from 'react';
import { Icon, type IconName } from './Icon';
import { shareMeta } from '../domain/metadata';
import type { ShareMode, User } from '../domain/types';

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

export function compactTime(value: string): string {
  if (value === '방금 전' || value === '오늘' || value === '어제' || value.endsWith('h') || value.endsWith('d')) return value;
  if (value.includes('05-06')) return '5d';
  if (value.includes('05-04')) return '1w';
  if (value.includes('05-02')) return '11d';
  return value.replace(/^2026-/, '').replace('-', '/');
}

export function byId<T extends { id: string }>(items: T[], id: string): T {
  const item = items.find((candidate) => candidate.id === id);
  if (!item) throw new Error(`Missing item: ${id}`);
  return item;
}

export function InfoRow({ icon, title, meta, children }: { icon: IconName; title: string; meta: string; children: ReactNode }) { return <article className="cw-info-row"><IconPocket tone="neutral" icon={icon} /><div><b>{title}</b><p>{children}</p></div><span>{meta}</span></article>; }

export function ActivityRow({ title, date, children }: { title: string; date: string; children: ReactNode }) { return <article className="cw-activity-row"><span><Icon name="recap" /></span><div><b>{title}</b><p>{children}</p></div><time>{date}</time></article>; }

export function SharePill({ mode, compact = false }: { mode: ShareMode; compact?: boolean }) { return <span className={`cw-share-pill ${shareMeta[mode].className}`}><Icon name={shareMeta[mode].icon} size={compact ? 11 : 12} />{compact ? shareMeta[mode].shortLabel : shareMeta[mode].label}</span>; }

export function ShareSelect({ mode, onChange }: { mode: ShareMode; onChange: (mode: ShareMode) => void }) { return <label className={`cw-share-select ${shareMeta[mode].className}`}><Icon name={shareMeta[mode].icon} /><select value={mode} onChange={(event) => onChange(event.target.value as ShareMode)}>{(Object.keys(shareMeta) as ShareMode[]).map((key) => <option key={key} value={key}>{shareMeta[key].label}</option>)}</select></label>; }

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
