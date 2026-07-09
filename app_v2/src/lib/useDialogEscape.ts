// Modal-stack ESC handler.
//
// Problem: many pages (files, project home, etc.) attach window-level keydown
// listeners that respond to Escape (clear selection, close menu, exit picker).
// When a dialog opens on top, both the dialog's onClose AND the page's
// background handler fire on the same Escape press, so the user sees the
// selection clear while the dialog stays open (or vice versa).
//
// Fix: dialogs register on a single capture-phase listener. The topmost
// non-disabled dialog wins via stopImmediatePropagation, which prevents every
// other window-level keydown handler — including page-level shortcut handlers
// — from running for this event. Background pages don't have to be aware of
// dialogs; they just keep their handlers as-is.

import { useEffect, useRef } from 'react';

type Entry = {
  onClose: () => void;
  isDisabled: () => boolean;
};

const stack: Entry[] = [];
let listenerAttached = false;

function handle(e: KeyboardEvent) {
  if (e.key !== 'Escape') return;
  // Walk top-down so the most recently opened dialog wins. Skip dialogs that
  // signal disabled (e.g. while a mutation is pending) — fall through to the
  // next one in the stack rather than blocking ESC entirely.
  for (let i = stack.length - 1; i >= 0; i--) {
    const entry = stack[i]!;
    if (!entry.isDisabled()) {
      e.stopImmediatePropagation();
      e.preventDefault();
      entry.onClose();
      return;
    }
  }
}

function ensureListener() {
  if (listenerAttached) return;
  window.addEventListener('keydown', handle, true);
  listenerAttached = true;
}

export function useDialogEscape(onClose: () => void, opts: { disabled?: boolean } = {}) {
  const disabled = !!opts.disabled;
  // Refs let the long-lived stack entry read the latest closures without
  // re-pushing on every render (which would scramble stack order).
  const onCloseRef = useRef(onClose);
  const disabledRef = useRef(disabled);
  onCloseRef.current = onClose;
  disabledRef.current = disabled;

  useEffect(() => {
    const entry: Entry = {
      onClose: () => onCloseRef.current(),
      isDisabled: () => disabledRef.current,
    };
    stack.push(entry);
    ensureListener();
    return () => {
      const idx = stack.indexOf(entry);
      if (idx >= 0) stack.splice(idx, 1);
    };
  }, []);
}
