import { describe, expect, it } from 'vitest';

import { buildHighlightSegments } from './mentionHighlight';

describe('buildHighlightSegments', () => {
  it('returns nothing for empty text', () => {
    expect(buildHighlightSegments('', [[0, 1]])).toEqual([]);
  });

  it('returns a single plain segment when there are no ranges', () => {
    expect(buildHighlightSegments('hello world', [])).toEqual([{ text: 'hello world', mention: false }]);
  });

  it('splits a single mention in the middle', () => {
    // "hi @jane ok" — mention spans indices 3..8
    expect(buildHighlightSegments('hi @jane ok', [[3, 8]])).toEqual([
      { text: 'hi ', mention: false },
      { text: '@jane', mention: true },
      { text: ' ok', mention: false },
    ]);
  });

  it('handles a mention at the very start and end', () => {
    expect(buildHighlightSegments('@jane', [[0, 5]])).toEqual([{ text: '@jane', mention: true }]);
  });

  it('renders multiple mentions in order', () => {
    // "@a and @b"
    expect(buildHighlightSegments('@a and @b', [[0, 2], [7, 9]])).toEqual([
      { text: '@a', mention: true },
      { text: ' and ', mention: false },
      { text: '@b', mention: true },
    ]);
  });

  it('merges overlapping and adjacent ranges', () => {
    // Confirmed mention [0,5) plus the active token [3,7) overlap.
    expect(buildHighlightSegments('@jane@x rest', [[0, 5], [3, 7]])).toEqual([
      { text: '@jane@x', mention: true },
      { text: ' rest', mention: false },
    ]);
  });

  it('clamps out-of-bounds ranges', () => {
    expect(buildHighlightSegments('@jane', [[0, 99]])).toEqual([{ text: '@jane', mention: true }]);
  });

  it('accepts unsorted ranges', () => {
    expect(buildHighlightSegments('@b @a', [[3, 5], [0, 2]])).toEqual([
      { text: '@b', mention: true },
      { text: ' ', mention: false },
      { text: '@a', mention: true },
    ]);
  });
});
