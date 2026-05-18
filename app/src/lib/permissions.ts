// Client-side permission predicates that mirror backend-v2's authz rules.
// These are UX hints only — backend enforces the actual security boundary.

import type { Project, Session, User } from '@/domain/types';

/**
 * Mirrors backend `require_owner` for `POST /projects/{id}/members`.
 * Every project — including the auto-created "Personal" one — accepts
 * invites from its owner. There is no special-case for personal projects.
 */
export function canInviteMembers(
  project: Project | null | undefined,
  currentUser: User | null | undefined,
): boolean {
  if (!project || !currentUser) return false;
  return project.ownerId === currentUser.id;
}

/**
 * Mirrors backend `DELETE /projects/{id}/members/{user_id}`:
 *  - owner can remove anyone except themselves (transfer ownership first)
 *  - member can only remove themselves (leave)
 */
export function canRemoveMember(
  project: Project | null | undefined,
  targetUser: User,
  currentUser: User | null | undefined,
): boolean {
  if (!project || !currentUser) return false;
  // Nobody can remove the owner this way; backend returns 400 either way.
  if (targetUser.id === project.ownerId) return false;
  if (project.ownerId === currentUser.id) return true; // owner removes others
  return targetUser.id === currentUser.id;             // self-leave
}

export function canLeaveProject(
  project: Project | null | undefined,
  currentUser: User | null | undefined,
): boolean {
  if (!project || !currentUser) return false;
  return project.ownerId !== currentUser.id;
}

/**
 * Mirrors backend `SessionAccess::Admin`:
 * the current user may delete/admin a session if they are
 *   (a) the session creator, OR
 *   (b) the project owner.
 *
 * If we don't know the project (e.g. sidebar before project query resolves)
 * we fall back to the creator-only check — backend will still 403 on mismatch.
 */
export function canAdministerSession(
  session: Session | null | undefined,
  project: Project | null | undefined,
  currentUser: User | null | undefined,
): boolean {
  if (!session || !currentUser) return false;
  if (session.creatorId === currentUser.id) return true;
  if (project && project.ownerId === currentUser.id) return true;
  return false;
}
