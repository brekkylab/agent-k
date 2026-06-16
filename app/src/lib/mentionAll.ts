// '@all' / '@모두' — a reserved mention token meaning "everyone in the project".
// It is expanded into the concrete member-id list at send time (frontend-only),
// so the rest of the mention pipeline (scanMentions, the team-message endpoint,
// the json_each unread/badge query) treats it as a set of ordinary mentions and
// needs no special casing. The sentinel rides through scanMentions as if it were
// a userId; the route translates it to the member list afterward.

export const ALL_MENTION_SENTINEL = '__all__';

// Canonical recognized literals. The popup inserts the locale-preferred one, but
// both are recognized regardless of UI language — message text is the source of
// truth and is rendered verbatim across locales.
export const ALL_MENTION_TOKENS = ['all', '모두'] as const;

// Add the reserved all-tokens to a username→id candidate map, but ONLY for
// tokens NOT already owned by a real member. scanMentions sorts keys
// longest-first (NOT by insertion order) and a Map holds one value per key, so
// blindly setting an owned key would overwrite that member and make them
// un-mentionable. Skipping an owned token keeps "@<handle>" reaching just them;
// the everyone feature stays available via the other, non-colliding token. (If a
// project somehow owns both tokens, the everyone feature is simply unavailable
// there — never a silent everyone-blast.)
export function withAllMentionKeys(map: ReadonlyMap<string, string>): Map<string, string> {
  const next = new Map(map);
  for (const token of ALL_MENTION_TOKENS) {
    if (!next.has(token)) next.set(token, ALL_MENTION_SENTINEL);
  }
  return next;
}

// Translate a scan result's user ids into the concrete recipient list: if the
// all-sentinel was matched, replace it with every member id except the sender
// (you don't notify yourself with @all); otherwise pass ids through. Explicit
// mentions are not self-filtered, matching the single-'@' behavior. The output
// never contains the sentinel, is deduped, and `hadAll` lets the caller keep a
// '@모두' draft a team message even when the expansion is empty (solo project).
export function expandAllMentions(
  userIds: readonly string[],
  allMemberIds: readonly string[],
  excludeId?: string,
): { userIds: string[]; hadAll: boolean } {
  const hadAll = userIds.includes(ALL_MENTION_SENTINEL);
  const out: string[] = [];
  const seen = new Set<string>();
  const add = (id: string) => {
    if (seen.has(id)) return;
    seen.add(id);
    out.push(id);
  };
  for (const id of userIds) {
    if (id === ALL_MENTION_SENTINEL) {
      for (const memberId of allMemberIds) {
        if (memberId !== excludeId) add(memberId);
      }
    } else {
      add(id);
    }
  }
  return { userIds: out, hadAll };
}
