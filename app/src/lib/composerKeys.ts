// Composer keyboard intent, factored out of the session page so the
// Enter/Shift+Enter branching is unit-testable without a DOM.
// Escape-to-stop lives in a window listener (the textarea is disabled mid-run),
// not here — see the session page.

export type ComposerKeyAction = 'send' | 'none';

export interface ComposerKeyEvent {
  key: string;
  shiftKey: boolean;
  isComposing: boolean;
}

// Enter sends; Shift+Enter inserts a newline. isComposing guards Korean/IME
// composition so confirming a character with Enter doesn't fire a send.
export function resolveComposerKeyAction(e: ComposerKeyEvent): ComposerKeyAction {
  if (e.isComposing) return 'none';
  if (e.key === 'Enter' && !e.shiftKey) return 'send';
  return 'none';
}
