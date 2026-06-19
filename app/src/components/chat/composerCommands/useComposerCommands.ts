import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, RefObject } from 'react';

import { detectCommandToken } from '@/lib/composerCommandToken';
import type { ActiveCommandToken, CommandTrigger } from '@/lib/composerCommandToken';

import type { CommandItem, ComposerCommand } from './types';

const MAX_VISIBLE_ITEMS = 20;

interface UseComposerCommandsOptions {
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  value: string;
  onChangeValue: (next: string) => void;
  commands: ComposerCommand[];
  disabled?: boolean;
}

export interface ComposerCommandsApi {
  open: boolean;
  items: CommandItem[];
  highlightIndex: number;
  activeCommand: ComposerCommand | null;
  /** The token currently being typed (trigger + [start, end)), or null. Lets
   *  callers highlight the in-progress token, e.g. a mention being composed. */
  activeToken: { trigger: string; start: number; end: number } | null;
  listboxId: string;
  activeOptionId: string | undefined;
  // Wire into the textarea. handleKeyDown returns true when the popup consumed
  // the key — callers must early-return before their own key handling.
  handleKeyDown: (e: KeyboardEvent<HTMLTextAreaElement>) => boolean;
  handleSelectionChange: () => void;
  handleCompositionStart: () => void;
  handleCompositionEnd: () => void;
  // Wire into the popup.
  selectItem: (index: number) => void;
  setHighlightIndex: (index: number) => void;
}

export function useComposerCommands({
  textareaRef,
  value,
  onChangeValue,
  commands,
  disabled = false,
}: UseComposerCommandsOptions): ComposerCommandsApi {
  const listboxId = useId();
  // Caret position mirrored into state so token detection stays a pure
  // function of (value, caret). null = selection is a range, not a caret.
  const [caret, setCaret] = useState<number | null>(null);
  const [highlightIndex, setHighlightIndex] = useState(0);
  // Escape marks the current token dismissed; the same token (same trigger at
  // the same index) won't reopen until the user moves out of it.
  const [dismissedAt, setDismissedAt] = useState<{ start: number; trigger: string } | null>(null);
  const composingRef = useRef(false);

  const triggers: CommandTrigger[] = useMemo(
    () => commands.map((c) => ({ char: c.trigger, position: c.triggerPosition })),
    [commands],
  );

  const token: ActiveCommandToken | null = useMemo(() => {
    if (disabled || caret === null) return null;
    return detectCommandToken(value, caret, triggers);
  }, [disabled, value, caret, triggers]);

  const activeCommand = useMemo(
    () => (token ? (commands.find((c) => c.trigger === token.trigger) ?? null) : null),
    [token, commands],
  );

  const dismissed =
    token !== null &&
    dismissedAt !== null &&
    dismissedAt.start === token.start &&
    dismissedAt.trigger === token.trigger;

  const open = token !== null && activeCommand !== null && !dismissed;

  const items = useMemo(
    () => (open && token && activeCommand ? activeCommand.getItems(token.query).slice(0, MAX_VISIBLE_ITEMS) : []),
    [open, token, activeCommand],
  );

  // Reset the highlight whenever the token target changes; drop a stale
  // dismissal once the user has left the dismissed token.
  const tokenKey = token ? `${token.trigger}:${token.start}:${token.query}` : '';
  useEffect(() => {
    setHighlightIndex(0);
  }, [tokenKey]);
  useEffect(() => {
    if (dismissedAt && (!token || token.start !== dismissedAt.start || token.trigger !== dismissedAt.trigger)) {
      setDismissedAt(null);
    }
  }, [token, dismissedAt]);

  const handleSelectionChange = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    setCaret(el.selectionStart === el.selectionEnd ? el.selectionStart : null);
  }, [textareaRef]);

  const selectItem = useCallback(
    (index: number) => {
      // Never rewrite the textarea while an IME composition is in flight —
      // the commit would corrupt the composed text.
      if (composingRef.current) return;
      if (!token || !activeCommand) return;
      const item = items[index];
      if (!item) return;
      const { replaceWith } = activeCommand.onSelect(item);
      const next = value.slice(0, token.start) + replaceWith + value.slice(token.end);
      onChangeValue(next);
      const pos = token.start + replaceWith.length;
      setCaret(pos);
      // The textarea is controlled; restore the caret after React re-renders.
      requestAnimationFrame(() => {
        const el = textareaRef.current;
        if (!el) return;
        el.focus();
        el.setSelectionRange(pos, pos);
      });
    },
    [token, activeCommand, items, value, onChangeValue, textareaRef],
  );

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>): boolean => {
      if (!open || e.nativeEvent.isComposing) return false;
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        if (items.length > 0) {
          const delta = e.key === 'ArrowDown' ? 1 : -1;
          setHighlightIndex((prev) => (prev + delta + items.length) % items.length);
        }
        return true;
      }
      if ((e.key === 'Enter' || e.key === 'Tab') && items.length > 0) {
        e.preventDefault();
        selectItem(highlightIndex);
        return true;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        if (token) setDismissedAt({ start: token.start, trigger: token.trigger });
        return true;
      }
      return false;
    },
    [open, items, highlightIndex, selectItem, token],
  );

  const handleCompositionStart = useCallback(() => {
    composingRef.current = true;
  }, []);
  const handleCompositionEnd = useCallback(() => {
    composingRef.current = false;
  }, []);

  // Close on pointerdown outside the popup and the textarea. Clicks inside
  // the textarea move the caret, which re-derives the token on its own.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: globalThis.PointerEvent) => {
      const target = e.target as Element | null;
      if (!target) return;
      if (target.closest(`[id="${listboxId}"]`)) return;
      if (textareaRef.current && target === textareaRef.current) return;
      if (token) setDismissedAt({ start: token.start, trigger: token.trigger });
    };
    document.addEventListener('pointerdown', onPointerDown);
    return () => document.removeEventListener('pointerdown', onPointerDown);
  }, [open, listboxId, textareaRef, token]);

  return {
    open,
    items,
    highlightIndex,
    activeCommand,
    activeToken: open && token ? { trigger: token.trigger, start: token.start, end: token.end } : null,
    listboxId,
    activeOptionId: open && items[highlightIndex] ? `${listboxId}-opt-${highlightIndex}` : undefined,
    handleKeyDown,
    handleSelectionChange,
    handleCompositionStart,
    handleCompositionEnd,
    selectItem,
    setHighlightIndex,
  };
}
