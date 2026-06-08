import { describe, expect, it } from 'vitest';

import { resolveComposerKeyAction, type ComposerKeyEvent } from './composerKeys';

const ev = (over: Partial<ComposerKeyEvent> = {}): ComposerKeyEvent => ({
  key: 'a',
  shiftKey: false,
  isComposing: false,
  ...over,
});

describe('resolveComposerKeyAction', () => {
  it('Enter sends', () => {
    expect(resolveComposerKeyAction(ev({ key: 'Enter' }))).toBe('send');
  });

  it('Shift+Enter inserts a newline (no send)', () => {
    expect(resolveComposerKeyAction(ev({ key: 'Enter', shiftKey: true }))).toBe('none');
  });

  it('Enter during IME composition is a no-op', () => {
    expect(resolveComposerKeyAction(ev({ key: 'Enter', isComposing: true }))).toBe('none');
  });

  it.each(['a', 'Escape', 'Tab', 'ArrowUp', 'Backspace', ' '])('%s is a no-op', (key) => {
    expect(resolveComposerKeyAction(ev({ key }))).toBe('none');
  });
});
