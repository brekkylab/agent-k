// Silent auto-login: deduplicated, signup-on-first-run, no interactive login screen.

import { request, setToken } from './client';
import type { LoginResponse } from './types';

const USERNAME = import.meta.env.VITE_LOCAL_USERNAME ?? 'local';
// Default password satisfies backend MIN_PASSWORD_LEN = 8 chars.
const PASSWORD = import.meta.env.VITE_LOCAL_PASSWORD ?? 'local-local';

// Shared inflight promise — concurrent callers share a single login attempt.
let inflight: Promise<boolean> | null = null;

async function doLogin(): Promise<boolean> {
  try {
    // Try login first.
    try {
      const resp = await request<LoginResponse>('/auth/login', {
        method: 'POST',
        body: { username: USERNAME, password: PASSWORD },
        skipAuth: true,
      });
      setToken(resp.access_token);
      return true;
    } catch (loginErr: unknown) {
      // If not 401, re-throw — unexpected error.
      if (
        typeof loginErr !== 'object' ||
        loginErr === null ||
        !('status' in loginErr) ||
        (loginErr as { status: number }).status !== 401
      ) {
        throw loginErr;
      }
    }

    // Login returned 401 — account may not exist yet. Try signup.
    try {
      await request('/auth/signup', {
        method: 'POST',
        body: { username: USERNAME, password: PASSWORD },
        skipAuth: true,
      });
    } catch (signupErr: unknown) {
      // 409: account exists but password is wrong — cannot recover.
      if (
        typeof signupErr === 'object' &&
        signupErr !== null &&
        'status' in signupErr &&
        (signupErr as { status: number }).status === 409
      ) {
        console.error(
          '[auth] account exists but password mismatch — cannot auto-login.' +
          ' Check VITE_LOCAL_USERNAME / VITE_LOCAL_PASSWORD.',
        );
        return false;
      }
      throw signupErr;
    }

    // Signup succeeded — now log in.
    const resp = await request<LoginResponse>('/auth/login', {
      method: 'POST',
      body: { username: USERNAME, password: PASSWORD },
      skipAuth: true,
    });
    setToken(resp.access_token);
    return true;
  } finally {
    inflight = null;
  }
}

/**
 * Ensure the local user is logged in.
 * Concurrent callers share one inflight request — fetch is called only once.
 */
export function ensureLogin(): Promise<boolean> {
  if (!inflight) {
    inflight = doLogin();
  }
  return inflight;
}
