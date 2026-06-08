import { describe, expect, it } from 'vitest';

import {
  resolveComposerKeyAction,
  type ComposerKeyEvent,
  type ComposerKeyState,
} from './composerKeys';

const ev = (over: Partial<ComposerKeyEvent> = {}): ComposerKeyEvent => ({
  key: 'a',
  shiftKey: false,
  isComposing: false,
  ...over,
});

const st = (over: Partial<ComposerKeyState> = {}): ComposerKeyState => ({
  streaming: false,
  ownedRunId: null,
  stopping: false,
  ...over,
});

// A run this tab owns, mid-stream, not yet stopping — the only state in which
// Escape should cut the run.
const streamingOwned = st({ streaming: true, ownedRunId: 'run-1', stopping: false });

describe('resolveComposerKeyAction', () => {
  describe('send', () => {
    it('Enter sends', () => {
      expect(resolveComposerKeyAction(ev({ key: 'Enter' }), st())).toBe('send');
    });

    it('Enter sends even while a run is streaming', () => {
      expect(resolveComposerKeyAction(ev({ key: 'Enter' }), streamingOwned)).toBe('send');
    });

    it('Shift+Enter inserts a newline (no send)', () => {
      expect(resolveComposerKeyAction(ev({ key: 'Enter', shiftKey: true }), st())).toBe('none');
    });

    it('Enter during IME composition is a no-op', () => {
      expect(resolveComposerKeyAction(ev({ key: 'Enter', isComposing: true }), st())).toBe('none');
    });
  });

  describe('stop on Escape', () => {
    it('stops when streaming, run owned, and not already stopping', () => {
      expect(resolveComposerKeyAction(ev({ key: 'Escape' }), streamingOwned)).toBe('stop');
    });

    it('does nothing when not streaming', () => {
      expect(
        resolveComposerKeyAction(ev({ key: 'Escape' }), st({ streaming: false, ownedRunId: 'run-1' })),
      ).toBe('none');
    });

    it('does nothing when the run is not owned by this tab', () => {
      expect(
        resolveComposerKeyAction(ev({ key: 'Escape' }), st({ streaming: true, ownedRunId: null })),
      ).toBe('none');
    });

    it('does nothing when a stop is already in flight', () => {
      expect(
        resolveComposerKeyAction(ev({ key: 'Escape' }), { ...streamingOwned, stopping: true }),
      ).toBe('none');
    });

    it('does nothing during IME composition even with a stoppable run', () => {
      expect(
        resolveComposerKeyAction(ev({ key: 'Escape', isComposing: true }), streamingOwned),
      ).toBe('none');
    });

    it('ignores Shift+Escape modifier the same as Escape (still stops)', () => {
      expect(
        resolveComposerKeyAction(ev({ key: 'Escape', shiftKey: true }), streamingOwned),
      ).toBe('stop');
    });
  });

  describe('other keys', () => {
    it.each(['a', 'Tab', 'ArrowUp', 'Backspace', ' '])('%s is a no-op while streaming', (key) => {
      expect(resolveComposerKeyAction(ev({ key }), streamingOwned)).toBe('none');
    });
  });
});
