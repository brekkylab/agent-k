// Splits composer text into plain / mention segments for the backdrop
// highlight overlay. Pure + DOM-free so the splitting logic is unit-testable;
// the overlay component just maps segments to spans.

export interface HighlightSegment {
  text: string;
  mention: boolean;
}

// `ranges` are [start, end) spans to highlight (confirmed mentions + the
// in-progress '@' token). Overlapping/adjacent ranges are merged so the
// output alternates plain/mention cleanly.
export function buildHighlightSegments(
  text: string,
  ranges: ReadonlyArray<readonly [number, number]>,
): HighlightSegment[] {
  if (text.length === 0) return [];
  const clamped = ranges
    .map(([s, e]) => [Math.max(0, s), Math.min(text.length, e)] as [number, number])
    .filter(([s, e]) => e > s)
    .sort((a, b) => a[0] - b[0]);

  const merged: Array<[number, number]> = [];
  for (const [s, e] of clamped) {
    const last = merged[merged.length - 1];
    if (last && s <= last[1]) {
      last[1] = Math.max(last[1], e);
    } else {
      merged.push([s, e]);
    }
  }

  if (merged.length === 0) return [{ text, mention: false }];

  const segments: HighlightSegment[] = [];
  let cursor = 0;
  for (const [s, e] of merged) {
    if (s > cursor) segments.push({ text: text.slice(cursor, s), mention: false });
    segments.push({ text: text.slice(s, e), mention: true });
    cursor = e;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor), mention: false });
  return segments;
}
