/// Short, URL-safe form of a session UUID.
///
/// Drops the canonical 8-4-4-4-12 hyphens and keeps the first 12 hex chars,
/// giving a `aaaaaaaa2222`-style id that is dense in the URL bar while still
/// providing ~281T collisions worth of space (16^12). The backend's prefix
/// lookup re-inserts hyphens when matching against the stored UUID form.
export function shortSessionId(fullId: string): string {
  return fullId.replace(/-/g, '').slice(0, 12);
}
