// Prominent mode switch above the capability grid — the ceiling's "No limit" and
// the member grant's "Inherit ceiling". Styled as a full-width highlighted card
// (accent border + wash) led by an always-visible switch, so it reads as a primary
// toggle rather than getting lost as a bare inline checkbox above the styled grid.

export function AgentModeToggle({
  label,
  hint,
  checked,
  disabled = false,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <label
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 11,
        width: '100%',
        marginBottom: 14,
        padding: '11px 13px',
        border: `1px solid ${checked ? 'var(--cw-selected-border)' : 'var(--cw-line)'}`,
        borderRadius: 10,
        background: checked ? 'var(--cw-selected-bg)' : 'var(--cw-paper)',
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.5 : 1,
        transition: 'border-color 120ms, background 120ms',
      }}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        style={{ width: 0, height: 0, opacity: 0, position: 'fixed', pointerEvents: 'none' }}
      />
      {/* Always-visible switch — signals "toggleable" in both states (the off
          state otherwise reads as static info). */}
      <span
        aria-hidden="true"
        style={{
          position: 'relative',
          flexShrink: 0,
          width: 38,
          height: 22,
          borderRadius: 999,
          background: checked ? 'var(--cw-accent)' : 'var(--cw-line)',
          transition: 'background 120ms',
        }}
      >
        <span
          style={{
            position: 'absolute',
            top: 2,
            left: 2,
            width: 18,
            height: 18,
            borderRadius: '50%',
            background: '#fff',
            boxShadow: '0 1px 2px rgba(0,0,0,0.2)',
            transform: checked ? 'translateX(16px)' : 'translateX(0)',
            transition: 'transform 140ms',
          }}
        />
      </span>
      <span style={{ minWidth: 0, flex: 1 }}>
        <span style={{ display: 'block', fontSize: 13.5, fontWeight: 600, color: 'var(--cw-ink)' }}>
          {label}
        </span>
        {hint && (
          <span style={{ display: 'block', marginTop: 2, fontSize: 12, color: 'var(--cw-ink-3)', lineHeight: 1.45 }}>
            {hint}
          </span>
        )}
      </span>
    </label>
  );
}
