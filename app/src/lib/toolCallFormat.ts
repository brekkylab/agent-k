// Tool-call value formatting — parses a tool call's arguments/result into a
// render-agnostic shape so the on-screen rows (ToolCallValueView) show structured
// key/value pairs instead of raw JSON, and arbitrary values are never run through a
// markdown parser. Pure & framework-free so it can be unit-tested in isolation.

/// Coerces a value (or a JSON-encoded string) into a plain object, or null if it isn't one.
export function asPlainObject(value: unknown): Record<string, unknown> | null {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  if (typeof value === 'string') {
    try {
      const parsed = JSON.parse(value);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>;
      }
    } catch {
      /* not JSON — fall through */
    }
  }
  return null;
}

/// A tool-call value parsed once into a render-agnostic shape.
export type ToolCallValue =
  | { kind: 'fields'; fields: { key: string; value: unknown }[] }
  | { kind: 'empty' }
  | { kind: 'raw'; text: string };

export function parseToolCallValue(value: unknown): ToolCallValue {
  const obj = asPlainObject(value);
  if (obj) {
    const entries = Object.entries(obj);
    if (entries.length === 0) return { kind: 'empty' };
    return { kind: 'fields', fields: entries.map(([key, v]) => ({ key, value: v })) };
  }
  // Object coercion failed (rare — the message API rejects invalid JSON upstream).
  return { kind: 'raw', text: typeof value === 'string' ? value : JSON.stringify(value, null, 2) };
}

/// How a single field value should be presented:
///  - 'inline': a bare scalar (number/boolean/null) shown next to the key
///  - 'code':   a single-line string shown as inline code (verbatim, no quotes)
///  - 'block':  a multi-line string, or a nested object/array, shown as a block.
///              Objects/arrays are pretty-printed so nesting stays readable.
export type FieldDisplay =
  | { kind: 'inline'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'block'; text: string };

export function classifyFieldValue(value: unknown): FieldDisplay {
  if (typeof value === 'string') {
    // Empty string would render as a blank value (just "key:"), indistinguishable
    // from a missing/unrendered field — show it explicitly as "".
    if (value === '') return { kind: 'code', text: '""' };
    return value.includes('\n') ? { kind: 'block', text: value } : { kind: 'code', text: value };
  }
  if (value !== null && typeof value === 'object') {
    // Render empty containers explicitly as {} / [] instead of an empty pretty block.
    const isEmpty = Array.isArray(value) ? value.length === 0 : Object.keys(value).length === 0;
    if (isEmpty) return { kind: 'inline', text: Array.isArray(value) ? '[]' : '{}' };
    return { kind: 'block', text: JSON.stringify(value, null, 2) };
  }
  return { kind: 'inline', text: JSON.stringify(value) }; // null → "null", numbers, booleans
}
