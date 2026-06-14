import { describe, expect, it } from 'vitest';

import { asPlainObject, parseToolCallValue, classifyFieldValue } from './toolCallFormat';

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
