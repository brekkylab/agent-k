import { describe, expect, it } from 'vitest';

import { detectCommandToken, type CommandTrigger } from './composerCommandToken';

const TRIGGERS: CommandTrigger[] = [
  { char: '#', position: 'word-boundary' },
  { char: '@', position: 'word-boundary' },
];

const SLASH: CommandTrigger[] = [{ char: '/', position: 'line-start' }];

describe('detectCommandToken', () => {
  it('detects a trigger at input start with an empty query', () => {
    expect(detectCommandToken('#', 1, TRIGGERS)).toEqual({ trigger: '#', query: '', start: 0, end: 1 });
  });

  it('detects a trigger after whitespace and captures the query up to the caret', () => {
    expect(detectCommandToken('hello #rep', 10, TRIGGERS)).toEqual({
      trigger: '#',
      query: 'rep',
      start: 6,
      end: 10,
    });
  });

  it('detects a trigger after a newline', () => {
    expect(detectCommandToken('line1\n@ji', 9, TRIGGERS)).toEqual({
      trigger: '@',
      query: 'ji',
      start: 6,
      end: 9,
    });
  });

  it('captures the query only up to a mid-token caret', () => {
    expect(detectCommandToken('#report', 4, TRIGGERS)).toEqual({
      trigger: '#',
      query: 'rep',
      start: 0,
      end: 4,
    });
  });

  it('supports a Korean query', () => {
    expect(detectCommandToken('#보고서', 4, TRIGGERS)).toEqual({
      trigger: '#',
      query: '보고서',
      start: 0,
      end: 4,
    });
  });

  it('rejects a mid-word trigger (emails, paths)', () => {
    expect(detectCommandToken('foo@bar', 7, TRIGGERS)).toBeNull();
    expect(detectCommandToken('a#b', 3, TRIGGERS)).toBeNull();
  });

  it('dismisses once whitespace follows the trigger', () => {
    expect(detectCommandToken('#rep done', 9, TRIGGERS)).toBeNull();
    expect(detectCommandToken('#rep ', 5, TRIGGERS)).toBeNull();
  });

  it('rejects a token containing another trigger char', () => {
    expect(detectCommandToken('@#x', 3, TRIGGERS)).toBeNull();
    expect(detectCommandToken('#a@b', 4, TRIGGERS)).toBeNull();
  });

  it('rejects queries over the 64-char cap', () => {
    const text = `#${'a'.repeat(65)}`;
    expect(detectCommandToken(text, text.length, TRIGGERS)).toBeNull();
    const ok = `#${'a'.repeat(64)}`;
    expect(detectCommandToken(ok, ok.length, TRIGGERS)).not.toBeNull();
  });

  it('returns null when the caret sits before any trigger', () => {
    expect(detectCommandToken('#rep', 0, TRIGGERS)).toBeNull();
    expect(detectCommandToken('', 0, TRIGGERS)).toBeNull();
  });

  it('line-start triggers fire at input start or after a newline only', () => {
    expect(detectCommandToken('/cmd', 4, SLASH)).toEqual({ trigger: '/', query: 'cmd', start: 0, end: 4 });
    expect(detectCommandToken('a\n/cmd', 6, SLASH)).toEqual({ trigger: '/', query: 'cmd', start: 2, end: 6 });
    // After a plain space is a word boundary but not a line start.
    expect(detectCommandToken('a /cmd', 6, SLASH)).toBeNull();
  });
});
