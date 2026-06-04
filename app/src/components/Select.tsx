import {
  Fragment,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import { Icon, type IconName } from './Icon';

// A single selectable value. `label` is always a plain string so the
// component can use it for the default render, the measurement clone, and the
// accessible name; richer visuals go through `renderOption` / `renderTrigger`.
export type SelectOption<T> = {
  value: T;
  label: string;
  icon?: IconName;
  // Applied to the trigger when this option is selected, and to the option
  // row — the lever for per-value styling (e.g. ShareSelect's mode colors).
  className?: string;
  disabled?: boolean;
};

// Optional grouping (renders a non-interactive header above its options).
export type SelectGroup<T> = {
  label?: string;
  options: SelectOption<T>[];
};

type SelectProps<T> = {
  value: T;
  onChange: (value: T) => void;
  options: SelectOption<T>[] | SelectGroup<T>[];
  // Escape hatches — override the default icon+label rendering.
  renderTrigger?: (option: SelectOption<T>, state: { open: boolean }) => ReactNode;
  renderOption?: (
    option: SelectOption<T>,
    state: { selected: boolean; focused: boolean },
  ) => ReactNode;
  // Extra class on the trigger button (state-dependent styling).
  triggerClassName?: string;
  // Extra class on the wrapper — use to size the select as a flex/grid item
  // (the trigger sizing/look goes on triggerClassName).
  className?: string;
  // Transition the trigger to the *current* label's intrinsic width via a
  // hidden measurement clone. Off by default; opt in for value-dependent
  // widths (ShareSelect). Utility selects usually leave this off.
  adaptiveWidth?: boolean;
  ariaLabel?: string;
  disabled?: boolean;
  // Shown when `value` matches no option (defensive; consumers normally pass a
  // value that exists in `options`).
  placeholder?: string;
  // Keep our trigger (styling + adaptive width) but open the *native* OS
  // dropdown instead of our custom panel — via a transparent real <select>
  // overlaid on the trigger. Gives native mobile pickers / OS list behavior.
  nativeDropdown?: boolean;
  // CSS selector for the ancestor the panel is kept *inside* (in addition to
  // the viewport). The panel is portaled to <body> so it's never clipped, but
  // by default it's only collision-corrected against the viewport — which lets
  // it spill across a pane boundary (e.g. a session dropdown drifting over the
  // members column). Pass a selector (matched via trigger.closest) to confine
  // it to that container instead. Omitted → viewport only.
  boundary?: string;
};

function isGrouped<T>(
  options: SelectOption<T>[] | SelectGroup<T>[],
): options is SelectGroup<T>[] {
  return options.length > 0 && 'options' in options[0];
}

export function Select<T>({
  value,
  onChange,
  options,
  renderTrigger,
  renderOption,
  triggerClassName = '',
  className = '',
  adaptiveWidth = false,
  ariaLabel,
  disabled = false,
  placeholder,
  nativeDropdown = false,
  boundary,
}: SelectProps<T>) {
  const groups: SelectGroup<T>[] = isGrouped(options) ? options : [{ options }];
  const flat = groups.flatMap((group) => group.options);
  const enabledIndices = flat
    .map((option, index) => (option.disabled ? -1 : index))
    .filter((index) => index >= 0);

  const selected = flat.find((option) => option.value === value);
  const selectedIdx = flat.findIndex((option) => option.value === value);

  const grouped = groups.some((group) => group.label);

  const [open, setOpen] = useState(false);
  const [focusIdx, setFocusIdx] = useState(() => (selectedIdx >= 0 ? selectedIdx : enabledIndices[0] ?? 0));
  // Whether the user has navigated by keyboard since opening. Drives the
  // keyboard focus highlight, so on open nothing is pre-highlighted (the
  // selected row shows only its check); the highlight follows real interaction.
  const [kbActive, setKbActive] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const measureRef = useRef<HTMLSpanElement>(null);
  const panelRef = useRef<HTMLUListElement>(null);
  const [width, setWidth] = useState<number | null>(null);
  // Collision-aware placement for the body-portaled panel. The panel is
  // position:fixed, so we compute its viewport coordinates here: flip above
  // when there's no room below, shift left when it would overrun the right
  // edge, and cap the height to the available space on the chosen side.
  const [placement, setPlacement] = useState<{
    side: 'top' | 'bottom';
    top: number;
    left: number;
    minWidth: number;
    maxHeight: number;
  }>({ side: 'bottom', top: -9999, left: -9999, minWidth: 0, maxHeight: 280 });

  // Stable ids so the trigger can point at the active option via
  // aria-activedescendant (announced by screen readers without moving focus).
  const baseId = useId();
  const listboxId = `${baseId}-listbox`;
  const optionId = (index: number) => `${baseId}-opt-${index}`;

  // Typeahead buffer: typing letters jumps focus to the first matching option
  // (native <select> parity). Buffer clears after a short idle gap.
  const typeahead = useRef<{ buffer: string; timer: number }>({ buffer: '', timer: 0 });

  // Adaptive width: mirror the measurement clone's intrinsic width onto the
  // real trigger. A ResizeObserver keeps it correct across label changes,
  // font swaps, and locale switches without the component knowing about i18n.
  useLayoutEffect(() => {
    if (!adaptiveWidth || !measureRef.current) return;
    const node = measureRef.current;
    const apply = () => setWidth(Math.ceil(node.getBoundingClientRect().width));
    apply();
    const observer = new ResizeObserver(apply);
    observer.observe(node);
    return () => observer.disconnect();
  }, [adaptiveWidth]);

  // Outside-click dismissal, mounted only while open. Keyboard lives on the
  // trigger's onKeyDown (the trigger keeps DOM focus while open), so a document
  // keydown listener isn't needed and would re-read the keypress that opened
  // the panel.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node;
      // The panel is portaled to <body>, so it's outside wrapRef — check both.
      if (!wrapRef.current?.contains(target) && !panelRef.current?.contains(target)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open]);

  // Position the body-portaled panel in the viewport. Recomputed on open and
  // while scrolling/resizing so it stays glued to the trigger; keeps the same
  // object reference when nothing changes so dependent effects don't churn.
  useLayoutEffect(() => {
    if (!open) return;
    const compute = () => {
      const trigger = triggerRef.current;
      const panel = panelRef.current;
      if (!trigger || !panel) return;
      const rect = trigger.getBoundingClientRect();
      const margin = 8; // keep this far from the box edges
      const gap = 6; // space between trigger and panel
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      // Collision box = viewport, intersected with the boundary container (if
      // any) so the panel stays inside its pane instead of drifting across it.
      // Each edge is inset by `margin` for breathing room.
      let boxLeft = margin;
      let boxRight = vw - margin;
      let boxTop = margin;
      let boxBottom = vh - margin;
      if (boundary) {
        const container = trigger.closest(boundary);
        if (container) {
          const b = container.getBoundingClientRect();
          boxLeft = Math.max(boxLeft, b.left + margin);
          boxRight = Math.min(boxRight, b.right - margin);
          boxTop = Math.max(boxTop, b.top + margin);
          boxBottom = Math.min(boxBottom, b.bottom - margin);
        }
      }
      const below = boxBottom - rect.bottom;
      const above = rect.top - boxTop;
      // Cap the height to ~N.5 rows so the last visible row is half-cut — a
      // "there's more, scroll" hint (the MUI pattern). Row height is measured
      // so it stays correct across font/density changes.
      const VISIBLE_ROWS = 9;
      const optionRow = panel.querySelector<HTMLElement>('[role=option]');
      const rowH = optionRow ? optionRow.getBoundingClientRect().height : 28;
      const vPad = 8; // panel padding-top + padding-bottom
      const cap = Math.round((VISIBLE_ROWS + 0.5) * rowH + vPad);
      // The height the panel actually wants (its content, capped).
      const desired = Math.min(cap, panel.scrollHeight);
      // Vertical: flip up only when it won't fit below — independent of how much
      // room is above (mirrors a native <select>'s "open down unless it can't").
      const side: 'top' | 'bottom' = below < desired ? 'top' : 'bottom';
      const maxHeight = Math.max(120, Math.min(cap, side === 'top' ? above : below));
      const panelH = Math.min(panel.scrollHeight, maxHeight);
      const top = side === 'bottom'
        ? Math.round(rect.bottom + gap)
        : Math.round(rect.top - gap - panelH);
      // Horizontal: align the panel's left to the trigger, but pull it back in
      // when it would overrun the right edge (mirrors the vertical flip). The
      // panel is ≥ the trigger width (minWidth) and may be wider for long labels.
      const minWidth = Math.round(rect.width);
      const panelW = Math.max(panel.getBoundingClientRect().width, rect.width);
      let left = rect.left;
      if (left + panelW > boxRight) left = boxRight - panelW;
      if (left < boxLeft) left = boxLeft;
      left = Math.round(left);
      setPlacement((prev) =>
        prev.side === side && prev.top === top && prev.left === left
        && prev.minWidth === minWidth && prev.maxHeight === maxHeight
          ? prev
          : { side, top, left, minWidth, maxHeight },
      );
    };
    compute();
    window.addEventListener('resize', compute);
    window.addEventListener('scroll', compute, true);
    return () => {
      window.removeEventListener('resize', compute);
      window.removeEventListener('scroll', compute, true);
    };
  }, [open, boundary]);

  // Keep the focused option in view as roving focus moves (and on open, so the
  // current selection is visible even far down a long/grouped list).
  useEffect(() => {
    if (!open) return;
    panelRef.current
      ?.querySelector<HTMLElement>(`[data-idx="${focusIdx}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [open, focusIdx, placement]);

  function commit(option: SelectOption<T>) {
    if (option.disabled) return;
    onChange(option.value);
    setOpen(false);
    triggerRef.current?.focus();
  }

  function openPanel() {
    setFocusIdx(selectedIdx >= 0 && !flat[selectedIdx]?.disabled ? selectedIdx : enabledIndices[0] ?? 0);
    setKbActive(false);
    setOpen(true);
  }

  // Move roving focus to the next/previous enabled option, wrapping around.
  function step(direction: 1 | -1) {
    if (enabledIndices.length === 0) return;
    setKbActive(true);
    const pos = enabledIndices.indexOf(focusIdx);
    const next = (pos + direction + enabledIndices.length) % enabledIndices.length;
    setFocusIdx(enabledIndices[next]);
  }

  // Type-to-jump: accumulate keystrokes and focus the first enabled option
  // whose label starts with the buffer; the buffer resets after an idle gap.
  function typeaheadTo(char: string) {
    setKbActive(true);
    const ta = typeahead.current;
    window.clearTimeout(ta.timer);
    ta.buffer += char.toLowerCase();
    ta.timer = window.setTimeout(() => { ta.buffer = ''; }, 600);
    const match = enabledIndices.find((i) => flat[i].label.toLowerCase().startsWith(ta.buffer));
    if (match != null) setFocusIdx(match);
  }

  function isTypeaheadChar(event: KeyboardEvent<HTMLButtonElement>) {
    return event.key.length === 1 && event.key !== ' ' && !event.ctrlKey && !event.metaKey && !event.altKey;
  }

  function onTriggerKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (disabled) return;
    if (!open) {
      if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        openPanel();
      } else if (isTypeaheadChar(event)) {
        openPanel();
        typeaheadTo(event.key);
      }
      return;
    }
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        step(1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        step(-1);
        break;
      case 'Home':
        event.preventDefault();
        if (enabledIndices.length) { setKbActive(true); setFocusIdx(enabledIndices[0]); }
        break;
      case 'End':
        event.preventDefault();
        if (enabledIndices.length) { setKbActive(true); setFocusIdx(enabledIndices[enabledIndices.length - 1]); }
        break;
      case 'Enter':
      case ' ':
        event.preventDefault();
        if (flat[focusIdx]) commit(flat[focusIdx]);
        break;
      case 'Escape':
        event.preventDefault();
        setOpen(false);
        break;
      case 'Tab':
        setOpen(false);
        break;
      default:
        if (isTypeaheadChar(event)) {
          event.preventDefault();
          typeaheadTo(event.key);
        }
    }
  }

  const triggerContent = selected
    ? renderTrigger
      ? renderTrigger(selected, { open })
      : (
        <>
          {selected.icon && <Icon name={selected.icon} />}
          <span>{selected.label}</span>
        </>
      )
    : <span>{placeholder ?? ''}</span>;

  // Same class string on the real trigger and the measurement clone, so the
  // clone's box (padding / gap / font / border) matches and its intrinsic
  // width is exactly what the trigger needs.
  const triggerClass = ['cw-select', selected?.className, triggerClassName]
    .filter(Boolean)
    .join(' ');

  // Native-dropdown mode: our visual-only trigger with a transparent real
  // <select> overlaid on top (see the `nativeDropdown` prop).
  if (nativeDropdown) {
    return (
      <div
        ref={wrapRef}
        className={`cw-select-wrap cw-select-wrap--native ${className}`.trim()}
        style={adaptiveWidth && width != null ? { width } : undefined}
      >
        <button
          type="button"
          tabIndex={-1}
          aria-hidden="true"
          className={triggerClass}
          style={adaptiveWidth && width != null ? { width } : undefined}
        >
          {triggerContent}
          <span className="cw-select-caret" aria-hidden="true" />
        </button>
        {adaptiveWidth && (
          <span ref={measureRef} className={`${triggerClass} cw-select-measure`} aria-hidden="true">
            {triggerContent}
            <span className="cw-select-caret" aria-hidden="true" />
          </span>
        )}
        <select
          className="cw-select-native-overlay"
          aria-label={ariaLabel}
          disabled={disabled}
          value={String(value)}
          onChange={(event) => {
            const next = flat.find((option) => String(option.value) === event.target.value);
            if (next && !next.disabled) onChange(next.value);
          }}
        >
          {groups.map((group) =>
            group.label ? (
              <optgroup key={group.label} label={group.label}>
                {group.options.map((option) => (
                  <option key={String(option.value)} value={String(option.value)} disabled={option.disabled}>
                    {option.label}
                  </option>
                ))}
              </optgroup>
            ) : (
              group.options.map((option) => (
                <option key={String(option.value)} value={String(option.value)} disabled={option.disabled}>
                  {option.label}
                </option>
              ))
            ),
          )}
        </select>
      </div>
    );
  }

  let flatIdx = -1;

  return (
    <div ref={wrapRef} className={`cw-select-wrap ${className}`.trim()}>
      <button
        ref={triggerRef}
        type="button"
        className={triggerClass}
        style={adaptiveWidth && width != null ? { width } : undefined}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-activedescendant={open && flat[focusIdx] ? optionId(focusIdx) : undefined}
        aria-label={ariaLabel}
        disabled={disabled}
        onClick={() => !disabled && (open ? setOpen(false) : openPanel())}
        onKeyDown={onTriggerKeyDown}
      >
        {triggerContent}
        <span className="cw-select-caret" aria-hidden="true" />
      </button>

      {adaptiveWidth && (
        <span ref={measureRef} className={`${triggerClass} cw-select-measure`} aria-hidden="true">
          {triggerContent}
          <span className="cw-select-caret" aria-hidden="true" />
        </span>
      )}

      {open && createPortal(
        <ul
          ref={panelRef}
          id={listboxId}
          role="listbox"
          className={`cw-select-panel ${placement.side === 'top' ? 'is-above' : 'is-below'}${grouped ? ' is-grouped' : ''}`}
          style={{
            top: placement.top,
            left: placement.left,
            minWidth: placement.minWidth,
            maxHeight: placement.maxHeight,
          }}
          aria-label={ariaLabel}
        >
          {groups.map((group, groupIdx) => (
            <Fragment key={group.label ?? groupIdx}>
              {group.label && (
                <li role="presentation" className="cw-select-group-label">
                  {group.label}
                </li>
              )}
              {group.options.map((option) => {
                flatIdx += 1;
                const idx = flatIdx;
                const isSelected = option.value === value;
                const isFocused = idx === focusIdx && kbActive;
                return (
                  <li
                    key={String(option.value)}
                    id={optionId(idx)}
                    data-idx={idx}
                    role="option"
                    aria-selected={isSelected}
                    aria-disabled={option.disabled || undefined}
                    className={
                      'cw-select-option' +
                      (option.className ? ` ${option.className}` : '') +
                      (isSelected ? ' is-selected' : '') +
                      (isFocused ? ' is-focused' : '') +
                      (option.disabled ? ' is-disabled' : '')
                    }
                    onMouseEnter={() => !option.disabled && setFocusIdx(idx)}
                    onClick={() => commit(option)}
                  >
                    {renderOption
                      ? renderOption(option, { selected: isSelected, focused: isFocused })
                      : (
                        <>
                          {option.icon && <Icon name={option.icon} />}
                          <span>{option.label}</span>
                          {isSelected && (
                            <span className="cw-select-check" aria-hidden="true">✓</span>
                          )}
                        </>
                      )}
                  </li>
                );
              })}
            </Fragment>
          ))}
        </ul>,
        document.body,
      )}
    </div>
  );
}
