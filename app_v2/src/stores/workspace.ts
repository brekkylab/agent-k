// Module singleton that holds the bootstrapped workspace ID.
// No zustand — plain module-level state is sufficient for a single-workspace app.
// The default workspace's id mirrors the user's id (backend create_default),
// so a single GET /me/workspace at bootstrap is enough to populate this.

let workspaceId: string | null = null;

export function setWorkspaceId(id: string): void {
  workspaceId = id;
}

/**
 * Returns the active workspace ID.
 * Throws if bootstrap has not run yet.
 */
export function getWorkspaceId(): string {
  if (workspaceId === null) {
    throw new Error('bootstrap has not run');
  }
  return workspaceId;
}
