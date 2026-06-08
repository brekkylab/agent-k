import { describe, expect, it } from 'vitest';
import { resolvePreviewKind, previewCodeLang } from './files';

describe('resolvePreviewKind', () => {
  it('maps pdf', () => expect(resolvePreviewKind('a.pdf')).toBe('pdf'));
  it('maps images', () => {
    for (const n of ['a.png', 'b.JPG', 'c.jpeg', 'd.gif', 'e.webp', 'f.svg', 'g.bmp', 'h.avif']) {
      expect(resolvePreviewKind(n)).toBe('image');
    }
  });
  it('maps html BEFORE code (critical: categorise lumps html into code)', () => {
    expect(resolvePreviewKind('page.html')).toBe('html');
    expect(resolvePreviewKind('page.htm')).toBe('html');
  });
  it('maps markdown', () => {
    expect(resolvePreviewKind('readme.md')).toBe('markdown');
    expect(resolvePreviewKind('x.markdown')).toBe('markdown');
  });
  it('maps code', () => {
    for (const n of ['a.ts', 'b.tsx', 'c.py', 'd.json', 'e.css', 'f.rs', 'g.go', 'h.yaml']) {
      expect(resolvePreviewKind(n)).toBe('code');
    }
  });
  it('maps tabular', () => {
    expect(resolvePreviewKind('data.csv')).toBe('table');
    expect(resolvePreviewKind('data.tsv')).toBe('table');
  });
  it('maps text', () => {
    for (const n of ['a.txt', 'b.log', 'd.env', 'e.ini']) {
      expect(resolvePreviewKind(n)).toBe('text');
    }
  });
  it('maps unsupported', () => {
    for (const n of ['a.docx', 'b.pptx', 'c.xlsx', 'd.zip', 'e.mp4', 'f.bin', 'noext']) {
      expect(resolvePreviewKind(n)).toBe('unsupported');
    }
  });
});

describe('previewCodeLang', () => {
  it('returns extension as highlight hint', () => {
    expect(previewCodeLang('a.ts')).toBe('ts');
    expect(previewCodeLang('b.PY')).toBe('py');
    expect(previewCodeLang('noext')).toBe('');
  });
});
