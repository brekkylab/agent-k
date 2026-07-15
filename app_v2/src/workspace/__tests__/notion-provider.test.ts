import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { FileStat } from 'webdav';

// The notion provider reads the mount over the WebDAV client; mock it (and the
// workspace store it reaches through) so tests drive a synthetic page tree.
const listDirectory = vi.fn();
const getFileText = vi.fn();
vi.mock('@/api/workspace', () => ({
  listDirectory: (...args: unknown[]) => listDirectory(...args),
  getFileText: (...args: unknown[]) => getFileText(...args),
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

import { makeNotionProvider } from '../providers/notion';
import { buildProviders, PROVIDERS } from '../providers';
import type { MountResponse } from '@/api/mounts';

function dir(basename: string, lastmod = '2026-07-01T00:00:00.000Z'): FileStat {
  return { filename: `/x/${basename}`, basename, type: 'directory', size: 0, lastmod } as FileStat;
}
function file(basename: string): FileStat {
  return { filename: `/x/${basename}`, basename, type: 'file', size: 0, lastmod: '' } as FileStat;
}

beforeEach(() => {
  listDirectory.mockReset();
  getFileText.mockReset();
});

describe('makeNotionProvider', () => {
  it('builds a page tree from directory listings with no page.json reads', async () => {
    // /notion/pages -> one root page; that page -> page.json + one child page.
    listDirectory.mockImplementation((path: string) => {
      if (path === '/notion/pages') return Promise.resolve([dir('Company_OS__root1', '2026-07-03T12:30:00.000Z')]);
      if (path === '/notion/pages/Company_OS__root1')
        return Promise.resolve([file('page.json'), dir('Hiring_Plan__child1', '2026-06-28T09:00:00.000Z')]);
      if (path === '/notion/pages/Company_OS__root1/Hiring_Plan__child1')
        return Promise.resolve([file('page.json')]);
      return Promise.resolve([]);
    });

    const provider = makeNotionProvider('/notion');
    const pages = await provider.list({});

    expect(getFileText).not.toHaveBeenCalled(); // tree built without content reads
    const root = pages.find((p) => p.id === 'root1');
    const child = pages.find((p) => p.id === 'child1');
    expect(root).toMatchObject({ title: 'Company OS', parentId: null, kind: 'page' });
    expect(child).toMatchObject({ title: 'Hiring Plan', parentId: 'root1', kind: 'page' });
  });

  it('reads page.json only in detail(), mapping markdown and workspace-parent to null', async () => {
    getFileText.mockResolvedValue(
      JSON.stringify({
        page_id: 'root1',
        title: 'Company OS',
        url: 'https://notion.so/root1',
        markdown: '# Company OS\n\nbody',
        parent_type: 'workspace',
        parent_id: '',
      }),
    );

    const provider = makeNotionProvider('notion'); // bare prefix accepted too
    const detail = await provider.detail('root1');

    expect(getFileText).toHaveBeenCalledWith('/notion/pages/x__root1/page.json');
    expect(detail.bodyPreview).toBe('# Company OS\n\nbody');
    expect(detail.externalUrl).toBe('https://notion.so/root1');
    expect(detail.entry.parentId).toBeNull();
  });
});

describe('buildProviders', () => {
  it('returns the static catalog when there are no mounts', () => {
    expect(buildProviders([])).toBe(PROVIDERS);
  });

  it('overlays a real Notion provider for a notion mount', () => {
    const mount: MountResponse = {
      id: 'm1',
      workspace_id: 'w1',
      prefix: '/notion',
      provider: { type: 'notion' },
      created_at: '',
      updated_at: '',
    };
    const providers = buildProviders([mount]);
    const notion = providers.find((p) => p.id === 'notion')!;
    // The real mount provider is connected and carries no fixture count, unlike
    // the static mock catalog entry.
    expect(notion.connected).toBe(true);
    expect(notion.count).toBeNull();
    // Non-notion entries are untouched (same references as the catalog).
    const localCatalog = PROVIDERS.find((p) => p.id === 'local');
    expect(providers.find((p) => p.id === 'local')).toBe(localCatalog);
  });
});
