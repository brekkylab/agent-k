// Composer keyboard intent, factored out of the session page so the
// Enter/Shift+Enter/Escape branching is unit-testable without a DOM.

export type ComposerKeyAction = 'send' | 'stop' | 'none';

export interface ComposerKeyEvent {
  key: string;
  shiftKey: boolean;
  isComposing: boolean;
}

export interface ComposerKeyState {
  streaming: boolean;
  ownedRunId: string | null;
  stopping: boolean;
}

// Enter sends; Shift+Enter inserts a newline; Escape stops this tab's run
// while it is streaming. isComposing guards Korean/IME composition so
// confirming or cancelling a character doesn't fire send/stop.
export function resolveComposerKeyAction(
  e: ComposerKeyEvent,
  state: ComposerKeyState,
): ComposerKeyAction {
  if (e.isComposing) return 'none';
  if (e.key === 'Enter' && !e.shiftKey) return 'send';
  if (e.key === 'Escape' && state.streaming && !!state.ownedRunId && !state.stopping) {
    return 'stop';
  }
  return 'none';
}
