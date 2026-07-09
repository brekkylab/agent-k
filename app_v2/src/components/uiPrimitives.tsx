import { type ReactNode } from 'react';
import { Icon, type IconName } from './Icon';

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

export function IconButton({
  icon,
  label,
  title,
  onClick,
  disabled = false,
  className,
  iconSize = 15,
  stopPropagation = false,
}: {
  icon: IconName;
  label: string;
  title?: string;
  onClick: () => void;
  disabled?: boolean;
  className?: string;
  iconSize?: number;
  stopPropagation?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={title ?? label}
      onClick={(e) => {
        if (stopPropagation) e.stopPropagation();
        if (disabled) return;
        onClick();
      }}
      disabled={disabled}
      className={`cw-icon-button${className ? ` ${className}` : ''}`}
    >
      <Icon name={icon} size={iconSize} />
    </button>
  );
}
