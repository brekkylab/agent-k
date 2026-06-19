// Inline-command token detection for the composer, factored out so the
// trigger/query parsing is unit-testable without a DOM (same rationale as
// composerKeys.ts). A token is the text between a trigger char ('#', '@', …)
// and the caret; the suggestion popup opens while a token is active.

export type TriggerPosition = 'word-boundary' | 'line-start';

export interface CommandTrigger {
  char: string;
  // 'word-boundary': valid at input start or after whitespace (default for
  // '#'/'@'). 'line-start': valid only at input start or after a newline —
  // reserved for future commands like '/'.
  position: TriggerPosition;
}

export interface ActiveCommandToken {
  trigger: string;
  query: string;
  // Index of the trigger char; the token spans [start, end).
  start: number;
  // Caret index. Replacing [start, end) is how a selection rewrites the text.
  end: number;
}

const MAX_QUERY_LENGTH = 64;

function isWhitespace(ch: string): boolean {
  return /\s/.test(ch);
}

// Detect the command token ending at the caret, or null when none is active.
// Callers must only invoke this with a collapsed selection (caret, no range).
export function detectCommandToken(
  text: string,
  caret: number,
  triggers: readonly CommandTrigger[],
): ActiveCommandToken | null {
  if (caret < 1 || caret > text.length) return null;

  const triggerChars = new Set(triggers.map((t) => t.char));

  // Walk backward from the caret. The nearest trigger char wins; whitespace
  // before any trigger means no token (typing a space dismisses the popup).
  for (let i = caret - 1; i >= 0; i--) {
    const ch = text[i];
    if (isWhitespace(ch)) return null;
    if (!triggerChars.has(ch)) continue;

    const trigger = triggers.find((t) => t.char === ch)!;
    const prev = i > 0 ? text[i - 1] : null;
    const positionOk =
      trigger.position === 'line-start'
        ? prev === null || prev === '\n'
        : prev === null || isWhitespace(prev);
    if (!positionOk) return null;

    const query = text.slice(i + 1, caret);
    if (query.length > MAX_QUERY_LENGTH) return null;
    return { trigger: ch, query, start: i, end: caret };
  }
  return null;
}
