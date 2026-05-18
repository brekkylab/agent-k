import type { ReactNode } from 'react';
import { Icon, type IconName } from './Icon';
import { intentMeta, shareMeta } from '../domain/metadata';
import type { SessionIntent, ShareMode, User } from '../domain/types';

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

export function IntentIcon({ intent, force = false }: { intent: SessionIntent; force?: boolean }) {
  if (intent === 'general' && !force) return <span className="cw-intent-dot" />;
  return <span className={`cw-pocket cw-intent-${intent}`}><Icon name={intentMeta[intent].icon} size={14} /></span>;
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

export function IntentBadge({ intent }: { intent: SessionIntent }) { return <span className="cw-intent-badge"><IntentIcon intent={intent} force />{intentMeta[intent].label}</span>; }

export function SharePill({ mode, compact = false }: { mode: ShareMode; compact?: boolean }) { return <span className={`cw-share-pill ${shareMeta[mode].className}`}><Icon name={shareMeta[mode].icon} size={compact ? 11 : 12} />{compact ? shareMeta[mode].shortLabel : shareMeta[mode].label}</span>; }

export function ShareSelect({ mode, onChange }: { mode: ShareMode; onChange: (mode: ShareMode) => void }) { return <label className={`cw-share-select ${shareMeta[mode].className}`}><Icon name={shareMeta[mode].icon} /><select value={mode} onChange={(event) => onChange(event.target.value as ShareMode)}>{(Object.keys(shareMeta) as ShareMode[]).map((key) => <option key={key} value={key}>{shareMeta[key].label}</option>)}</select></label>; }
