import { useEffect, useRef } from 'react';

import type { CommandItem } from './types';

interface CommandSuggestionPopupProps {
  items: CommandItem[];
  highlightIndex: number;
  listboxId: string;
  isLoading: boolean;
  emptyLabel: string;
  loadingLabel: string;
  /** Accessible name for the listbox (screen readers announce it on focus). */
  ariaLabel: string;
  onHighlight: (index: number) => void;
  onSelect: (index: number) => void;
}

// Presentational listbox anchored above the composer box (the box provides
// position: relative). Option rows use onPointerDown + preventDefault so the
// textarea keeps focus through a click-select.
export function CommandSuggestionPopup({
  items,
  highlightIndex,
  listboxId,
  isLoading,
  emptyLabel,
  loadingLabel,
  ariaLabel,
  onHighlight,
  onSelect,
}: CommandSuggestionPopupProps) {
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const el = listRef.current?.querySelector('[aria-selected="true"]');
    el?.scrollIntoView({ block: 'nearest' });
  }, [highlightIndex, items]);

  return (
    <div className="cw-cmd-popup" role="listbox" id={listboxId} ref={listRef} aria-label={ariaLabel}>
      {isLoading && items.length === 0 && <div className="cw-cmd-status">{loadingLabel}</div>}
      {!isLoading && items.length === 0 && <div className="cw-cmd-status">{emptyLabel}</div>}
      {items.map((item, i) => (
        <div
          key={item.id}
          id={`${listboxId}-opt-${i}`}
          role="option"
          aria-selected={i === highlightIndex}
          className="cw-cmd-item"
          onPointerDown={(e) => {
            e.preventDefault();
            onSelect(i);
          }}
          onPointerEnter={() => onHighlight(i)}
        >
          {item.icon && <span className="cw-cmd-icon">{item.icon}</span>}
          <span className="cw-cmd-label">{item.label}</span>
          {item.sublabel && <span className="cw-cmd-sub">{item.sublabel}</span>}
        </div>
      ))}
    </div>
  );
}
