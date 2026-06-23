// Shared toggle grid for the fixed agent-capability vocabulary. Used by the
// project-ceiling editor and the member's own-grant editor. Purely presentational
// — the parent owns the selected set and the draft/save lifecycle. Each cell is a
// whole-box toggle: the native checkbox is visually hidden (kept for a11y/keyboard)
// and selection is conveyed by the box's accent border/wash + a trailing check.

import { useTranslation } from 'react-i18next';
import { AGENT_CAPABILITY_GROUPS, capabilityIcon, capabilityLabelKey } from '@/domain/agentCapabilities';
import { Icon } from '@/components/Icon';

export function AgentCapabilityChecklist({
  ns,
  selected,
  onToggle,
  disabled = false,
  /**
   * The project ceiling, when this list is a member's own grant. Capabilities
   * outside it stay selectable (the choice is preserved), but are flagged as
   * "outside the ceiling" — checking one has no effect until the ceiling allows
   * it, because the backend intersects the grant with the ceiling. `null`/absent
   * = no ceiling context (nothing flagged), e.g. the ceiling editor itself.
   */
  allowed,
}: {
  /** i18n namespace holding the `agent_caps.*` labels. */
  ns: 'project';
  selected: ReadonlySet<string>;
  onToggle: (capability: string, next: boolean) => void;
  disabled?: boolean;
  allowed?: ReadonlySet<string> | null;
}) {
  const { t } = useTranslation(ns);
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {AGENT_CAPABILITY_GROUPS.map(({ group, capabilities }) => (
        <div key={group}>
          <div
            style={{
              fontSize: 11,
              fontWeight: 600,
              textTransform: 'uppercase',
              letterSpacing: '0.04em',
              color: 'var(--cw-ink-3)',
              marginBottom: 6,
            }}
          >
            {t(`agent_cap_groups.${group}`)}
          </div>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(4, minmax(0, 1fr))',
              gap: 6,
            }}
          >
            {capabilities.map((cap) => {
              // Outside the ceiling: still selectable + preserved, but inert until
              // the ceiling allows it. Only a read-only parent (`disabled`) blocks
              // toggling.
              const outOfCeiling = allowed != null && !allowed.has(cap);
              const checked = selected.has(cap);
              return (
                <label
                  key={cap}
                  title={outOfCeiling ? t('agent_grant.out_of_ceiling_hint') : undefined}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '7px 9px',
                    border: `1px solid ${checked ? 'var(--cw-selected-border)' : 'var(--cw-line)'}`,
                    borderRadius: 8,
                    background: checked ? 'var(--cw-selected-bg)' : 'var(--cw-paper)',
                    fontSize: 13,
                    color: 'var(--cw-ink)',
                    cursor: disabled ? 'default' : 'pointer',
                    opacity: disabled ? 0.5 : 1,
                    transition: 'border-color 120ms, background 120ms',
                  }}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={disabled}
                    onChange={(e) => onToggle(cap, e.target.checked)}
                    style={{
                      width: 0,
                      height: 0,
                      opacity: 0,
                      position: 'fixed',
                      pointerEvents: 'none',
                    }}
                  />
                  <Icon
                    name={capabilityIcon(cap)}
                    size={15}
                    style={{
                      flexShrink: 0,
                      color: outOfCeiling
                        ? 'var(--cw-ink-4)'
                        : checked
                          ? 'var(--cw-accent)'
                          : 'var(--cw-ink-3)',
                    }}
                  />
                  <span style={{ minWidth: 0, flex: 1 }}>
                    <span
                      style={
                        outOfCeiling
                          ? { textDecoration: 'line-through', color: 'var(--cw-ink-4)' }
                          : undefined
                      }
                    >
                      {t(`agent_caps.${capabilityLabelKey(cap)}`)}
                    </span>
                    {outOfCeiling && (
                      <span style={{ color: 'var(--cw-warn, #9a6700)', fontSize: 11, marginLeft: 6 }}>
                        {t('agent_grant.out_of_ceiling')}
                      </span>
                    )}
                  </span>
                  {checked && (
                    <Icon
                      name="check"
                      size={15}
                      style={{ flexShrink: 0, color: 'var(--cw-accent)' }}
                    />
                  )}
                </label>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}
