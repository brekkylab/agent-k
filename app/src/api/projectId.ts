// slug → project_id resolver shared by api/projects, api/dirents.
//
// Backend API contract is id-only; frontend URLs are slug-based. This shim
// lets the api client accept either form so call sites don't have to wire a
// useProject hook through to every leaf API call.
//
// Cache is process-lifetime — a slug-to-id mapping is stable until the
// project is renamed, in which case the retired-slug lookup still maps to
// the same project id, so cache hits stay correct.
import { getProjectBySlug } from './projects';

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const cache = new Map<string, string>();

export async function resolveProjectId(slugOrId: string): Promise<string> {
  if (UUID_RE.test(slugOrId)) return slugOrId;
  const cached = cache.get(slugOrId);
  if (cached) return cached;
  const project = await getProjectBySlug(slugOrId);
  cache.set(slugOrId, project.id);
  return project.id;
}
