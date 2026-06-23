// Segmented control (tablist) for a small set of mutually-exclusive choices —
// an inline alternative to a dropdown, in the cw-side-seg / ComposerAgentPicker
// tab style. Each option may carry an optional leading icon.

import { Icon, type IconName } from './Icon';

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  icon?: IconName;
}

export function SegmentedControl<T extends string>({
  value,
  onChange,
  options,
  ariaLabel,
  className,
  iconOnly = false,
}: {
  value: T;
  onChange: (value: T) => void;
  options: SegmentedOption<T>[];
  ariaLabel?: string;
  // Extra class on the track (e.g. to size it as a flex item).
  className?: string;
  // Icon-only variant: shrink to content and hide labels visually (kept for
  // screen readers). Requires every option to carry an `icon`.
  iconOnly?: boolean;
}) {
  const trackClass = [
    'cw-segmented',
    iconOnly ? 'cw-segmented--icononly' : '',
    className ?? '',
  ].filter(Boolean).join(' ');
  return (
    <div className={trackClass} role="tablist" aria-label={ariaLabel}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="tab"
          aria-selected={o.value === value}
          className="cw-segmented-tab"
          onClick={() => onChange(o.value)}
        >
          {o.icon && <Icon name={o.icon} size={14} />}
          <span>{o.label}</span>
        </button>
      ))}
    </div>
  );
}
