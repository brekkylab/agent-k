import { describe, expect, it, vi } from 'vitest';

// LocalProvider reaches webdav → workspaceClient() → getWorkspaceId(), which throws
// "bootstrap has not run" in tests. Mock BOTH modules (mirror the pattern in
// src/api/__tests__/workspace.test.ts) BEFORE importing the registry.
vi.mock('@/api/workspace', () => ({
  listDirectory: vi.fn().mockResolvedValue([]),
  getFileBlob: vi.fn(),
  putFile: vi.fn(),
  deleteEntry: vi.fn(),
  createDirectory: vi.fn(),
  workspaceClient: vi.fn(),
}));
vi.mock('@/stores/workspace', () => ({
  getWorkspaceId: vi.fn(() => 'test-wid'),
  setWorkspaceId: vi.fn(),
}));

import { PROVIDERS, allRecent, getProvider } from '../providers';

describe('provider registry', () => {
  it('registers 7 providers in rail order', () => {
    expect(PROVIDERS.map((p) => p.id)).toEqual(['local', 'gdrive', 's3', 'confluence', 'jira', 'gmail', 'slack']);
  });
  it('only local is attachable', () => {
    expect(PROVIDERS.filter((p) => p.attachable).map((p) => p.id)).toEqual(['local']);
  });
  it('groups by the three categories', () => {
    const cats = new Set(PROVIDERS.map((p) => p.category));
    expect(cats).toEqual(new Set(['files', 'docs', 'messages']));
  });
  it('allRecent merges newest-first across providers', async () => {
    const merged = await allRecent();
    expect(merged.length).toBeGreaterThan(10);
    const times = merged.map((e) => e.modifiedAt);
    expect([...times].sort().reverse()).toEqual(times);
    expect(new Set(merged.map((e) => e.sourceId)).size).toBeGreaterThanOrEqual(6);
  });
  it('mock provider list/detail round-trip', async () => {
    const jira = getProvider('jira')!;
    const list = await jira.list({});
    const d = await jira.detail(list[0].id);
    expect(d.entry.id).toBe(list[0].id);
    expect(d.bodyPreview).toBeTruthy();
  });
});
