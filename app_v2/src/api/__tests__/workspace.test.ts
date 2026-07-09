import { describe, it, expect, vi, beforeEach } from 'vitest';

// Mock the webdav module before importing workspace functions.
vi.mock('webdav', () => {
  const mockClient = {
    getDirectoryContents: vi.fn(),
    getFileContents: vi.fn(),
    putFileContents: vi.fn(),
    deleteFile: vi.fn(),
    createDirectory: vi.fn(),
  };
  return {
    AuthType: { None: 'None' },
    createClient: vi.fn(() => mockClient),
    _mockClient: mockClient,
  };
});

// Mock stores/workspace so getWorkspaceId() doesn't throw.
vi.mock('@/stores/workspace', () => ({
  getWorkspaceId: vi.fn(() => 'test-workspace-id'),
}));

// Mock api/client so reauthOnce is controllable.
vi.mock('@/api/client', () => ({
  BASE_URL: 'http://localhost:8080',
  getToken: vi.fn(() => 'test-token'),
  reauthOnce: vi.fn(async () => false),
}));

import { listDirectory, getFileBlob } from '../workspace';
import * as webdav from 'webdav';
import * as clientModule from '@/api/client';

function getMockClient() {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return (webdav as any)._mockClient as {
    getDirectoryContents: ReturnType<typeof vi.fn>;
    getFileContents: ReturnType<typeof vi.fn>;
    putFileContents: ReturnType<typeof vi.fn>;
    deleteFile: ReturnType<typeof vi.fn>;
    createDirectory: ReturnType<typeof vi.fn>;
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('listDirectory()', () => {
  it('normalizes array result to FileStat[]', async () => {
    const fakeEntries = [
      { filename: '/readme.md', basename: 'readme.md', type: 'file', size: 100 },
      { filename: '/docs', basename: 'docs', type: 'directory', size: 0 },
    ];
    getMockClient().getDirectoryContents.mockResolvedValue(fakeEntries);

    const result = await listDirectory('/');
    expect(result).toEqual(fakeEntries);
  });

  it('normalizes object-with-data result to array', async () => {
    const fakeEntries = [{ filename: '/file.txt', basename: 'file.txt', type: 'file', size: 42 }];
    getMockClient().getDirectoryContents.mockResolvedValue({ data: fakeEntries });

    const result = await listDirectory('/');
    expect(result).toEqual(fakeEntries);
  });

  it('calls reauth once on 401, then retries the operation', async () => {
    const reauthMock = vi.mocked(clientModule.reauthOnce);
    reauthMock.mockResolvedValue(true); // reauth succeeds

    const fakeEntries = [{ filename: '/x.txt', basename: 'x.txt', type: 'file', size: 1 }];
    getMockClient().getDirectoryContents
      .mockRejectedValueOnce({ status: 401, message: 'Unauthorized' })
      .mockResolvedValueOnce(fakeEntries);

    const createClientMock = vi.mocked(webdav.createClient);
    const result = await listDirectory('/');
    expect(reauthMock).toHaveBeenCalledOnce();
    expect(getMockClient().getDirectoryContents).toHaveBeenCalledTimes(2);
    expect(createClientMock).toHaveBeenCalledTimes(2);
    expect(result).toEqual(fakeEntries);
  });

  it('re-throws the error when reauth returns false on 401', async () => {
    const reauthMock = vi.mocked(clientModule.reauthOnce);
    reauthMock.mockResolvedValue(false);

    getMockClient().getDirectoryContents.mockRejectedValue({ status: 401, message: 'Unauthorized' });

    await expect(listDirectory('/')).rejects.toMatchObject({ status: 401 });
    expect(reauthMock).toHaveBeenCalledOnce();
  });
});

describe('getFileBlob()', () => {
  it('returns a Blob wrapping the binary ArrayBuffer contents', async () => {
    const bytes = new Uint8Array([72, 101, 108, 108, 111]); // "Hello" — 5 bytes
    getMockClient().getFileContents.mockResolvedValue(bytes.buffer);

    const blob = await getFileBlob('/hello.txt');
    // jsdom Blob does not implement .text()/.arrayBuffer(), so verify via size.
    expect(blob).toBeInstanceOf(Blob);
    expect(blob.size).toBe(5);
  });
});
