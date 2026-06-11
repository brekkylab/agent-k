import { describe, expect, it } from 'vitest';

import {
  asPlainObject,
  parseToolCallValue,
  classifyFieldValue,
  inlineCode,
  codeFence,
  fieldToMarkdown,
  valueToMarkdown,
  toolCallToMarkdown,
} from './toolCallFormat';

describe('asPlainObject', () => {
  it('returns plain objects as-is', () => {
    const o = { a: 1 };
    expect(asPlainObject(o)).toBe(o);
  });

  it('parses JSON-encoded objects', () => {
    expect(asPlainObject('{"a":1}')).toEqual({ a: 1 });
  });

  it('rejects arrays (top-level and JSON-encoded)', () => {
    expect(asPlainObject([1, 2])).toBeNull();
    expect(asPlainObject('[1,2]')).toBeNull();
  });

  it('rejects non-object scalars and plain strings', () => {
    expect(asPlainObject('hello')).toBeNull();
    expect(asPlainObject('42')).toBeNull(); // valid JSON, but a number
    expect(asPlainObject(42)).toBeNull();
    expect(asPlainObject(null)).toBeNull();
  });

  it('does not throw on invalid JSON strings', () => {
    expect(asPlainObject('{not json')).toBeNull();
  });
});

describe('parseToolCallValue', () => {
  it('maps a plain object to fields', () => {
    expect(parseToolCallValue({ path: '/a', limit: 5 })).toEqual({
      kind: 'fields',
      fields: [
        { key: 'path', value: '/a' },
        { key: 'limit', value: 5 },
      ],
    });
  });

  it('treats an empty object as empty', () => {
    expect(parseToolCallValue({})).toEqual({ kind: 'empty' });
  });

  it('expands JSON-string objects into fields', () => {
    expect(parseToolCallValue('{"a":1}')).toEqual({
      kind: 'fields',
      fields: [{ key: 'a', value: 1 }],
    });
  });

  it('falls back to raw (pretty JSON) for arrays', () => {
    expect(parseToolCallValue([1, 2])).toEqual({ kind: 'raw', text: '[\n  1,\n  2\n]' });
  });

  it('falls back to raw verbatim for plain strings', () => {
    expect(parseToolCallValue('just text')).toEqual({ kind: 'raw', text: 'just text' });
  });
});

describe('classifyFieldValue', () => {
  it('single-line string → code', () => {
    expect(classifyFieldValue('/path/to/x')).toEqual({ kind: 'code', text: '/path/to/x' });
  });

  it('multi-line string → block (verbatim)', () => {
    expect(classifyFieldValue('a\nb')).toEqual({ kind: 'block', text: 'a\nb' });
  });

  it('number / boolean / null → inline', () => {
    expect(classifyFieldValue(42)).toEqual({ kind: 'inline', text: '42' });
    expect(classifyFieldValue(true)).toEqual({ kind: 'inline', text: 'true' });
    expect(classifyFieldValue(null)).toEqual({ kind: 'inline', text: 'null' });
  });

  it('nested object → block with pretty-printed JSON (regression: not one-line)', () => {
    const d = classifyFieldValue({ a: 1, b: { c: 2 } });
    expect(d.kind).toBe('block');
    expect(d.text).toBe('{\n  "a": 1,\n  "b": {\n    "c": 2\n  }\n}');
  });

  it('array → block with pretty-printed JSON', () => {
    expect(classifyFieldValue([1, 2])).toEqual({ kind: 'block', text: '[\n  1,\n  2\n]' });
  });

  // Ambiguous empty values are rendered explicitly so they don't look like a
  // blank/missing field.
  it('empty string → explicit ""', () => {
    expect(classifyFieldValue('')).toEqual({ kind: 'code', text: '""' });
  });

  it('empty object → inline {}', () => {
    expect(classifyFieldValue({})).toEqual({ kind: 'inline', text: '{}' });
  });

  it('empty array → inline []', () => {
    expect(classifyFieldValue([])).toEqual({ kind: 'inline', text: '[]' });
  });

  it('null → inline null', () => {
    expect(classifyFieldValue(null)).toEqual({ kind: 'inline', text: 'null' });
  });
});

describe('inlineCode', () => {
  it('uses single backticks for plain content', () => {
    expect(inlineCode('hello')).toBe('`hello`');
  });

  it('widens the fence past inner backtick runs', () => {
    expect(inlineCode('a`b')).toBe('``a`b``');
    expect(inlineCode('a``b')).toBe('```a``b```');
  });

  it('pads with a space when content starts/ends with a backtick', () => {
    expect(inlineCode('`x')).toBe('`` `x ``');
    expect(inlineCode('x`')).toBe('`` x` ``');
  });
});

describe('codeFence', () => {
  it('uses a triple fence by default', () => {
    expect(codeFence('plain')).toBe('```\nplain\n```');
  });

  it('widens the fence past an inner triple-backtick run', () => {
    expect(codeFence('a\n```\nb')).toBe('````\na\n```\nb\n````');
  });
});

describe('fieldToMarkdown', () => {
  it('wraps single-line strings in inline code', () => {
    expect(fieldToMarkdown('path', '/a/b')).toBe('- **path**: `/a/b`');
  });

  it('keeps inline code intact when the value contains backticks', () => {
    expect(fieldToMarkdown('cmd', 'use `ls`')).toBe('- **cmd**: `` use `ls` ``');
  });

  it('prints scalars bare', () => {
    expect(fieldToMarkdown('limit', 5)).toBe('- **limit**: 5');
  });

  it('fences multi-line strings', () => {
    expect(fieldToMarkdown('body', 'line1\nline2')).toBe('- **body**:\n\n```\nline1\nline2\n```');
  });

  it('fences nested objects with pretty JSON', () => {
    expect(fieldToMarkdown('opts', { a: 1 })).toBe('- **opts**:\n\n```\n{\n  "a": 1\n}\n```');
  });
});

describe('valueToMarkdown', () => {
  it('renders fields as a bullet list', () => {
    expect(valueToMarkdown({ path: '/a', limit: 5 })).toBe('- **path**: `/a`\n- **limit**: 5');
  });

  it('renders empty objects as a marker', () => {
    expect(valueToMarkdown({})).toBe('_(empty)_');
  });

  it('renders raw fallback in a code block', () => {
    expect(valueToMarkdown('plain text')).toBe('```\nplain text\n```');
  });
});

describe('toolCallToMarkdown', () => {
  it('includes only the name when there are no args/result', () => {
    expect(toolCallToMarkdown({ name: 'ping' })).toBe('## 🔧 ping');
  });

  it('serializes name, inputs and results with section headers', () => {
    const md = toolCallToMarkdown({
      name: 'read',
      arguments: { path: '/x' },
      result: '{"error":"nope"}',
    });
    expect(md).toBe(
      [
        '## 🔧 read',
        '',
        '### Inputs',
        '',
        '- **path**: `/x`',
        '',
        '### Results',
        '',
        '- **error**: `nope`',
      ].join('\n'),
    );
  });

  it('omits a section when its value is undefined', () => {
    const md = toolCallToMarkdown({ name: 'read', arguments: { a: 1 } });
    expect(md).not.toContain('### Results');
    expect(md).toContain('### Inputs');
  });
});
