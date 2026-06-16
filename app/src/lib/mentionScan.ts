// Send-time mention extraction. The composer keeps a map of popup-selected
// candidates (username → userId); the message text stays the single source of
// truth, so mentions are re-derived here instead of being tracked statefully —
// deleting "@username" by hand naturally drops the mention.
//
// Usernames are free text at signup (spaces, dots, Korean are all legal), so
// matching is literal indexOf — no regex over user-controlled input. Keys are
// tried longest-first so "@jeffrey" wins over a shorter "@jeff" candidate, and
// matched spans are consumed to prevent overlapping hits.

export interface MentionScanResult {
  userIds: string[];
  usernames: string[];
  /** [start, end) text ranges of every matched "@username" token, in order.
   *  Unlike userIds, duplicates of the same user are each included (for
   *  highlighting). */
  ranges: Array<[number, number]>;
}

// A match must end at a boundary: end of text, or a char that cannot continue
// an identifier. "-"/"_" and letters/digits continue one, so "@kim-cs" does
// NOT match a "kim" candidate; "@kim." and "@kim cs" (literal key) do.
function isBoundary(ch: string | undefined): boolean {
  if (ch === undefined) return true;
  return !/[\p{L}\p{N}_-]/u.test(ch);
}

function isWhitespace(ch: string): boolean {
  return /\s/.test(ch);
}

export function scanMentions(
  text: string,
  candidates: ReadonlyMap<string, string>,
): MentionScanResult {
  const userIds: string[] = [];
  const usernames: string[] = [];
  if (candidates.size === 0 || !text.includes('@')) {
    return { userIds, usernames, ranges: [] };
  }

  const keys = [...candidates.keys()]
    .filter((k) => k.length > 0)
    .sort((a, b) => b.length - a.length);
  const consumed: Array<[number, number]> = [];
  const seenIds = new Set<string>();

  for (const key of keys) {
    const needle = `@${key}`;
    let from = 0;
    let at: number;
    while ((at = text.indexOf(needle, from)) !== -1) {
      from = at + 1;
      const end = at + needle.length;
      if (consumed.some(([s, e]) => at < e && end > s)) continue;
      // Mirror the trigger rule: '@' counts only at input start or after
      // whitespace, so "john@jeffrey.com" never mentions jeffrey.
      if (at > 0 && !isWhitespace(text[at - 1])) continue;
      if (!isBoundary(text[end])) continue;

      consumed.push([at, end]);
      const userId = candidates.get(key)!;
      if (!seenIds.has(userId)) {
        seenIds.add(userId);
        userIds.push(userId);
        usernames.push(key);
      }
    }
  }
  // consumed holds one range per matched token (longest-first scan order);
  // sort by start so highlighting can render them left to right.
  const ranges = consumed.slice().sort((a, b) => a[0] - b[0]);
  return { userIds, usernames, ranges };
}
