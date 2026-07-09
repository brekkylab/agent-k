// Tool-call value formatting — shared by the on-screen rows (ToolCallValueView)
// and the copy-to-clipboard markdown (toolCallToMarkdown). Both derive from one
// parse, so display and copy never drift, and arbitrary values are never run
// through a markdown parser (no escaping/interpretation/XSS bugs). Pure & framework
// -free so it can be unit-tested in isolation.

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

/// Longest run of consecutive backticks in `text` (0 if none).
function longestBacktickRun(text: string): number {
  return (text.match(/`+/g) ?? []).reduce((max, run) => Math.max(max, run.length), 0);
}

/// Wraps text in inline code with a backtick fence long enough to survive any backticks
/// in the content (CommonMark: fence longer than the longest inner run; pad with a space
/// when the content starts/ends with a backtick so the fences don't merge).
export function inlineCode(text: string): string {
  const fence = '`'.repeat(longestBacktickRun(text) + 1);
  const pad = text.startsWith('`') || text.endsWith('`') ? ' ' : '';
  return `${fence}${pad}${text}${pad}${fence}`;
}

/// Wraps text in a fenced code block, widening the fence past any ``` run inside it.
export function codeFence(text: string): string {
  const fence = '`'.repeat(Math.max(3, longestBacktickRun(text) + 1));
  return `${fence}\n${text}\n${fence}`;
}

/// Renders a single key/value as a markdown bullet, mirroring how the rows view
/// presents the same value (inline code / fenced block / bare scalar).
export function fieldToMarkdown(key: string, value: unknown): string {
  const d = classifyFieldValue(value);
  const k = `**${inlineCode(key)}**`;
  switch (d.kind) {
    case 'inline':
      return `- ${k}: ${d.text}`;
    case 'code':
      return `- ${k}: ${inlineCode(d.text)}`;
    case 'block':
      return `- ${k}:\n\n${codeFence(d.text)}`;
  }
}

/// Serializes a parsed value to markdown (for copy). Same structure as the rows view.
export function valueToMarkdown(value: unknown): string {
  const parsed = parseToolCallValue(value);
  switch (parsed.kind) {
    case 'empty':
      return '_(empty)_';
    case 'fields':
      return parsed.fields.map((f) => fieldToMarkdown(f.key, f.value)).join('\n');
    case 'raw':
      return codeFence(parsed.text);
  }
}

/// Serializes a tool call (name, inputs, results) into a readable markdown block for copying.
export function toolCallToMarkdown(tc: { name: string; arguments?: unknown; result?: string }): string {
  const lines = [`## 🔧 ${tc.name}`];
  if (tc.arguments !== undefined) {
    lines.push('', '### Inputs', '', valueToMarkdown(tc.arguments));
  }
  if (tc.result !== undefined) {
    lines.push('', '### Results', '', valueToMarkdown(tc.result));
  }
  return lines.join('\n');
}
