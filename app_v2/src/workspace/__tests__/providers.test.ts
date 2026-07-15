import { describe, expect, it, vi } from 'vitest';

// LocalProvider reaches webdav → workspaceClient() → getWorkspaceId(), which throws
// "bootstrap has not run" in tests. Mock BOTH modules (mirror the pattern in
// src/api/__tests__/workspace.test.ts) BEFORE importing the registry.
vi.mock('@/api/workspace', () => ({
  listDirectory: vi.fn().mockResolvedValue([]),
  getFileBlob: vi.fn(),
  getFileText: vi.fn(),
  putFile: vi.fn(),
  deleteEntry: vi.fn(),
  createDirectory: vi.fn(),
  workspaceClient: vi.fn(),
}));
vi.mock('@/stores/workspace', () => ({
  getWorkspaceId: vi.fn(() => 'test-wid'),
  setWorkspaceId: vi.fn(),
}));

import { PROVIDERS, recentAcross, getProviderMeta } from '../providers';
import enFiles from '@/i18n/locales/en/files.json';
import koFiles from '@/i18n/locales/ko/files.json';

describe('provider registry', () => {
  it('registers providers in rail order with Notion as a docs source', () => {
    expect(PROVIDERS.map((p) => p.id)).toEqual([
      'local',
      'gdrive',
      's3',
      'dropbox',
      'figma',
      'confluence',
      'notion',
      'jira',
      'github',
      'linear',
      'gmail',
      'slack',
      'knowledge',
    ]);
    expect(PROVIDERS.filter((p) => p.category === 'docs').map((p) => p.id)).toEqual([
      'confluence',
      'notion',
      'jira',
      'github',
      'linear',
    ]);
  });
  it('keeps catalog-only sources disconnected until connected (notion/s3 need a real mount)', () => {
    expect(PROVIDERS.filter((p) => !p.connected).map((p) => p.id)).toEqual([
      's3',
      'dropbox',
      'figma',
      'notion',
      'github',
      'linear',
    ]);
  });
  it('only local is attachable', () => {
    expect(PROVIDERS.filter((p) => p.attachable).map((p) => p.id)).toEqual(['local']);
  });
  it('labels the WebDAV-backed local provider as Shared Files', () => {
    expect(getProviderMeta('local')?.nameKey).toBe('workspace.src.local');
    expect(enFiles.workspace.src.local).toBe('Shared Files');
    expect(koFiles.workspace.src.local).toBe('Shared Files');
  });
  it('separates source categories from the knowledge layer', () => {
    const cats = new Set(PROVIDERS.map((p) => p.category));
    expect(cats).toEqual(new Set(['files', 'docs', 'messages', 'knowledge']));
    expect(PROVIDERS.filter((p) => p.category === 'knowledge').map((p) => p.id)).toEqual(['knowledge']);
  });
  it('recentAcross merges newest-first across providers', async () => {
    const merged = await recentAcross(PROVIDERS);
    expect(merged.length).toBeGreaterThan(10);
    const times = merged.map((e) => e.modifiedAt);
    expect([...times].sort().reverse()).toEqual(times);
    expect(new Set(merged.map((e) => e.sourceId)).size).toBeGreaterThanOrEqual(6);
  });
  it('mock provider list/detail round-trip', async () => {
    const jira = getProviderMeta('jira')!;
    const list = await jira.list({});
    const d = await jira.detail(list[0].id);
    expect(d.entry.id).toBe(list[0].id);
    expect(d.bodyPreview).toBeTruthy();
  });
  it('lists notion as a catalog-only pages source (no fixture data; real tree comes from a mount)', () => {
    // Notion is disconnected until a real mount overlays it; the page-tree
    // behavior is covered by notion-provider.test.ts (the real WebDAV provider).
    const notion = getProviderMeta('notion')!;
    expect(notion.kind).toBe('pages');
    expect(notion.connected).toBe(false);
  });
  it('provides curated knowledge records with provenance', async () => {
    const knowledge = getProviderMeta('knowledge')!;
    expect(knowledge.kind).toBe('records');

    const records = await knowledge.list({});
    expect(records.some((record) => record.kind === 'record' && record.status === 'approved')).toBe(true);
    expect(records.some((record) => (record.evidenceRefs ?? []).length > 1)).toBe(true);

    const detail = await knowledge.detail('knowledge-decision-q3-mobile-performance');
    expect(detail.bodyPreview).toContain('Approved workspace decision');
  });
});
