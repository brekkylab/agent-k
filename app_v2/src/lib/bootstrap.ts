// Ensures the app is authenticated and knows its default workspace before
// rendering routes.

import { getToken } from '@/api/client';
import { ensureLogin } from '@/api/auth';
import { getMyWorkspace } from '@/api/workspaces';
import { setWorkspaceId } from '@/stores/workspace';

// Cached promise — resolved on first run, reset on failure so the next navigation retries.
let cached: Promise<void> | null = null;

async function runBootstrap(): Promise<void> {
  try {
    // Step 1: ensure authenticated.
    if (!getToken()) {
      const ok = await ensureLogin();
      if (!ok) throw new Error('Authentication failed — cannot auto-login');
    }
    // If a stale token exists, proceed; mid-flight 401 retry in client.ts covers it.

    // Step 2: resolve the default workspace. Signup provisions one per user
    // (its id mirrors the user id), so a single fetch is enough — no
    // list-or-create dance.
    const workspace = await getMyWorkspace();

    // Step 3: store for the rest of the app to use.
    setWorkspaceId(workspace.id);
  } catch (err) {
    // Reset cache so the next call (e.g. page reload) retries from scratch.
    cached = null;
    throw err;
  }
}

/**
 * Idempotent bootstrap — runs once and caches the result.
 * On failure, the cache is cleared so a retry is possible.
 */
export function ensureBootstrap(): Promise<void> {
  if (!cached) {
    cached = runBootstrap();
  }
  return cached;
}
