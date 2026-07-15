import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { FileStat } from 'webdav';

// The S3 provider reads the mount over the WebDAV client; mock it (and the
// workspace store it reaches through) so tests drive a synthetic file tree.
const listDirectory = vi.fn();
vi.mock('@/api/workspace', () => ({
  listDirectory: (...args: unknown[]) => listDirectory(...args),
  getFileText: vi.fn(),
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

import { makeS3Provider } from '../providers/s3';
import { buildProviders, PROVIDERS } from '../providers';
import type { MountResponse } from '@/api/mounts';

function entry(name: string, type: 'file' | 'directory', lastmod = '2026-07-01T00:00:00.000Z'): FileStat {
  return { filename: `/s3-prod/${name}`, basename: name, type, size: type === 'file' ? 12 : 0, lastmod } as FileStat;
}

beforeEach(() => listDirectory.mockReset());

describe('makeS3Provider', () => {
  it('lists the mount root by default and maps stats to s3 entries', async () => {
    listDirectory.mockResolvedValue([entry('reports', 'directory'), entry('notes.txt', 'file')]);
    const provider = makeS3Provider('/s3-prod');

    const items = await provider.list({});
    expect(listDirectory).toHaveBeenCalledWith('/s3-prod'); // default = mount root
    expect(items.map((e) => [e.title, e.kind])).toEqual([
      ['reports', 'folder'],
      ['notes.txt', 'file'],
    ]);
    expect(items.every((e) => e.sourceId === 's3')).toBe(true);
    // Entries carry full workspace paths so navigation passes them straight back.
    expect(items[0].path).toBe('/s3-prod/reports');
  });

  it('navigates into a subdirectory via ctx.path (bare prefix accepted too)', async () => {
    listDirectory.mockResolvedValue([]);
    const provider = makeS3Provider('s3-prod');
    await provider.list({ path: '/s3-prod/reports' });
    expect(listDirectory).toHaveBeenCalledWith('/s3-prod/reports');
  });

  it('recent() returns only files, newest-first', async () => {
    listDirectory.mockResolvedValue([
      entry('a.txt', 'file', '2026-07-01T00:00:00.000Z'),
      entry('sub', 'directory', '2026-07-09T00:00:00.000Z'),
      entry('b.txt', 'file', '2026-07-05T00:00:00.000Z'),
    ]);
    const provider = makeS3Provider('/s3-prod');
    const recents = await provider.recent();
    expect(recents.map((e) => e.title)).toEqual(['b.txt', 'a.txt']); // no dirs, newest first
  });

  it('detail() resolves an entry by listing its parent', async () => {
    listDirectory.mockResolvedValue([entry('reports', 'directory'), entry('notes.txt', 'file')]);
    const provider = makeS3Provider('/s3-prod');
    const detail = await provider.detail('/s3-prod/notes.txt');
    expect(listDirectory).toHaveBeenCalledWith('/s3-prod'); // parent of the id
    expect(detail.entry).toMatchObject({ title: 'notes.txt', kind: 'file', sourceId: 's3' });
  });
});

describe('buildProviders (s3)', () => {
  it('overlays a real S3 provider for an s3 mount', () => {
    const mount: MountResponse = {
      id: 'm1',
      workspace_id: 'w1',
      prefix: '/s3-prod',
      provider: { type: 's3', bucket: 'my-bucket', region: 'us-east-1' },
      created_at: '',
      updated_at: '',
    };
    const providers = buildProviders([mount]);
    const s3 = providers.find((p) => p.id === 's3')!;
    // The real mount provider is connected and carries no fixture count, unlike
    // the static (disconnected) catalog entry.
    expect(s3.connected).toBe(true);
    expect(s3.count).toBeNull();
    // Non-s3 entries are untouched (same references as the catalog).
    expect(providers.find((p) => p.id === 'local')).toBe(PROVIDERS.find((p) => p.id === 'local'));
  });
});
