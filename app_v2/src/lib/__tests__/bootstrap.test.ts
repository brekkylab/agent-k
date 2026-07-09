import { describe, it, expect, vi, beforeEach } from 'vitest';
import { setToken } from '@/api/client';

// vi.mock calls are hoisted — declare them at file scope so they apply
// before any dynamic import of the module under test.
vi.mock('@/api/auth', () => ({
  ensureLogin: vi.fn(),
}));
vi.mock('@/api/workspaces', () => ({
  getMyWorkspace: vi.fn(),
}));

// Import the mocked modules so we can configure them per-test.
import { ensureLogin } from '@/api/auth';
import { getMyWorkspace } from '@/api/workspaces';

beforeEach(() => {
  localStorage.clear();
  setToken(null);
  // Reset all mock implementations between tests.
  vi.resetAllMocks();
  // Re-configure defaults after reset.
  vi.mocked(ensureLogin).mockResolvedValue(true);
});

describe('ensureBootstrap()', () => {
  it('resolves the default workspace → sets workspace id', async () => {
    setToken('existing-token');
    vi.mocked(getMyWorkspace).mockResolvedValue({
      id: 'ws-1',
      title: "alice's workspace",
      created_at: '',
      updated_at: '',
    });

    // Reset module cache and reimport to get a fresh `cached` state.
    vi.resetModules();
    const { ensureBootstrap } = await import('@/lib/bootstrap');
    await ensureBootstrap();

    const { getWorkspaceId } = await import('@/stores/workspace');
    expect(getWorkspaceId()).toBe('ws-1');
  });

  it('logs in first when no token is present, then sets workspace id', async () => {
    // No token → ensureLogin must run before the workspace fetch.
    vi.mocked(ensureLogin).mockResolvedValue(true);
    vi.mocked(getMyWorkspace).mockResolvedValue({
      id: 'ws-login',
      title: "bob's workspace",
      created_at: '',
      updated_at: '',
    });

    vi.resetModules();
    const { ensureBootstrap } = await import('@/lib/bootstrap');
    await ensureBootstrap();

    expect(ensureLogin).toHaveBeenCalledOnce();
    const { getWorkspaceId } = await import('@/stores/workspace');
    expect(getWorkspaceId()).toBe('ws-login');
  });

  it('failure rejects and a second call retries (cache reset)', async () => {
    setToken('existing-token');
    // First call fails, second succeeds.
    vi.mocked(getMyWorkspace)
      .mockRejectedValueOnce(new Error('network error'))
      .mockResolvedValueOnce({
        id: 'ws-retry',
        title: 'Retry',
        created_at: '',
        updated_at: '',
      });

    vi.resetModules();
    const { ensureBootstrap } = await import('@/lib/bootstrap');

    // First call should reject.
    await expect(ensureBootstrap()).rejects.toThrow('network error');

    // Second call: cache was reset on failure, so reimport is needed to get the same module instance.
    const { ensureBootstrap: ensureBootstrap2 } = await import('@/lib/bootstrap');
    await ensureBootstrap2();

    const { getWorkspaceId } = await import('@/stores/workspace');
    expect(getWorkspaceId()).toBe('ws-retry');
  });
});
