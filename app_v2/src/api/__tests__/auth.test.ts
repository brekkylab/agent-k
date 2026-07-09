import { describe, it, expect, vi, beforeEach } from 'vitest';
import { setToken, getToken } from '../client';

// We need to isolate module state, so reset between tests.
beforeEach(() => {
  localStorage.clear();
  setToken(null);
  vi.restoreAllMocks();
  vi.resetModules();
});

function makeResp(body: unknown, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('ensureLogin()', () => {
  it('login 200 → sets token, returns true', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
      makeResp({ access_token: 'tok1', token_type: 'bearer', expires_in: 3600, user: { id: '1', username: 'local', role: 'user', display_name: null, is_active: true, preferred_language: 'en', created_at: '', updated_at: '' } }, 200),
    ));
    const { ensureLogin } = await import('../auth');
    const ok = await ensureLogin();
    expect(ok).toBe(true);
    expect(getToken()).toBe('tok1');
  });

  it('login 401 → signup 201 → login 200 (3 fetch calls, in order)', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(makeResp({ error: 'bad creds' }, 401))
      .mockResolvedValueOnce(makeResp({ id: '1', username: 'local' }, 201))
      .mockResolvedValueOnce(makeResp({ access_token: 'tok2', token_type: 'bearer', expires_in: 3600, user: {} }, 200));
    vi.stubGlobal('fetch', fetchMock);

    const { ensureLogin } = await import('../auth');
    const ok = await ensureLogin();
    expect(ok).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(3);
    // Verify call order: login, signup, login.
    expect((fetchMock.mock.calls[0][0] as string)).toContain('/auth/login');
    expect((fetchMock.mock.calls[1][0] as string)).toContain('/auth/signup');
    expect((fetchMock.mock.calls[2][0] as string)).toContain('/auth/login');
  });

  it('signup 409 → returns false and logs error', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(makeResp({ error: 'bad creds' }, 401))
      .mockResolvedValueOnce(makeResp({ error: 'conflict' }, 409));
    vi.stubGlobal('fetch', fetchMock);
    const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

    const { ensureLogin } = await import('../auth');
    const ok = await ensureLogin();
    expect(ok).toBe(false);
    expect(consoleSpy).toHaveBeenCalled();
  });

  it('concurrent calls share one inflight — fetch called once', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      makeResp({ access_token: 'tok3', token_type: 'bearer', expires_in: 3600, user: {} }, 200),
    );
    vi.stubGlobal('fetch', fetchMock);

    const { ensureLogin } = await import('../auth');
    const [r1, r2, r3] = await Promise.all([ensureLogin(), ensureLogin(), ensureLogin()]);
    expect(r1).toBe(true);
    expect(r2).toBe(true);
    expect(r3).toBe(true);
    // Only one login fetch should have been made.
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
