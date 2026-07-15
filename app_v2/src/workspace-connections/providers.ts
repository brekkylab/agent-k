import type { SourceEntry, SourceProvider, SourceType } from './types';
import type { MountResponse } from '@/api/mounts';
import { localProvider } from './providers/local';
import { makeMockProvider } from './providers/mock';
import { makeNotionProvider } from './providers/notion';
import { makeS3Provider } from './providers/s3';
import * as gdrive from './fixtures/gdrive';
import * as confluence from './fixtures/confluence';
import * as jira from './fixtures/jira';
import * as gmail from './fixtures/gmail';
import * as slack from './fixtures/slack';
import * as knowledge from './fixtures/knowledge';

const emptyFixture = {
  entries: [],
  details: {},
};

// Static catalog: the connectable sources. `local` is real (WebDAV); the rest
// are mock fixtures until a real mount overlays them (see `buildProviders`).
// This list is also the source of truth for catalog membership — the router
// loader guard checks it synchronously via `getProviderMeta`.
export const PROVIDERS: SourceProvider[] = [
  localProvider,
  makeMockProvider({ id: 'gdrive', nameKey: 'workspace.src.gdrive', category: 'files', kind: 'files', count: gdrive.entries.length }, gdrive),
  // S3 is catalog-only until a real mount exists: `connected: false` routes to
  // the connect dialog (S3MountForm), like notion, instead of showing fixture
  // data. `buildProviders` overlays the real, connected provider once an S3
  // mount is registered.
  makeMockProvider({ id: 's3', nameKey: 'workspace.src.s3', category: 'files', kind: 'files', count: 0, connected: false }, emptyFixture),
  makeMockProvider({ id: 'dropbox', nameKey: 'workspace.src.dropbox', category: 'files', kind: 'files', count: 0, connected: false }, emptyFixture),
  makeMockProvider({ id: 'figma', nameKey: 'workspace.src.figma', category: 'files', kind: 'items', count: 0, connected: false }, emptyFixture),
  makeMockProvider({ id: 'confluence', nameKey: 'workspace.src.confluence', category: 'docs', kind: 'items', count: confluence.entries.length }, confluence),
  // Notion is catalog-only until a real mount exists: `connected: false` so it
  // routes to the connect dialog (NotionMountForm) like github/linear, instead
  // of showing fixture data. `buildProviders` overlays the real, connected
  // provider once a Notion mount is registered.
  makeMockProvider({ id: 'notion', nameKey: 'workspace.src.notion', category: 'docs', kind: 'pages', count: 0, connected: false }, emptyFixture),
  makeMockProvider({ id: 'jira', nameKey: 'workspace.src.jira', category: 'docs', kind: 'items', count: jira.entries.length }, jira),
  makeMockProvider({ id: 'github', nameKey: 'workspace.src.github', category: 'docs', kind: 'items', count: 0, connected: false }, emptyFixture),
  makeMockProvider({ id: 'linear', nameKey: 'workspace.src.linear', category: 'docs', kind: 'items', count: 0, connected: false }, emptyFixture),
  makeMockProvider({ id: 'gmail', nameKey: 'workspace.src.gmail', category: 'messages', kind: 'threads', count: gmail.entries.length }, gmail),
  makeMockProvider({ id: 'slack', nameKey: 'workspace.src.slack', category: 'messages', kind: 'threads', count: slack.entries.length }, slack),
  makeMockProvider({ id: 'knowledge', nameKey: 'workspace.src.knowledge', category: 'knowledge', kind: 'records', count: knowledge.entries.length }, knowledge),
];

// Synchronous catalog-membership lookup. Used by the router loader guard
// (`beforeLoad` runs outside React and cannot call the mount-aware hooks); it
// only needs "is this a known source id?", which never depends on mounts.
export function getProviderMeta(id: string): SourceProvider | undefined {
  return PROVIDERS.find((p) => p.id === id);
}

// Provider types that connect via a real workspace VFS mount (0..N instances
// each). Their static catalog entries are the add-dialog/hint placeholders only;
// the browseable providers are the per-mount instances from `buildProviders`.
export const MOUNT_BACKED_TYPES = new Set<SourceType>(['s3', 'notion']);

// Map a workspace mount to a real per-instance provider (id = mount.id).
function realProviderForMount(mount: MountResponse): SourceProvider | null {
  if (mount.provider.type === 'notion') return makeNotionProvider(mount);
  if (mount.provider.type === 's3') return makeS3Provider(mount);
  return null;
}

// One provider PER mount (instance-scoped), plus the static catalog MINUS the
// mount-backed placeholders (s3/notion) — those types are represented only by
// their real instances here; their catalog entries live on in `PROVIDERS` for
// the add-dialog type-picker + hint candidates. Pure function of `mounts` so
// callers memoize on the mounts list for stable provider identity.
export function buildProviders(mounts: MountResponse[]): SourceProvider[] {
  const instances = mounts
    .map(realProviderForMount)
    .filter((p): p is SourceProvider => p !== null);
  const base = PROVIDERS.filter((p) => !MOUNT_BACKED_TYPES.has(p.type));
  return [...base, ...instances];
}

// Merge newest-first recents across the connected providers. A per-provider
// failure is logged and treated as empty so one bad source can't blank the
// unified list.
export async function recentAcross(providers: SourceProvider[]): Promise<SourceEntry[]> {
  const results = await Promise.all(
    providers
      .filter((p) => p.connected)
      .map((p) =>
        p.recent().catch((err) => {
          console.warn(`[workspace] ${p.id} recent() failed`, err);
          return [] as SourceEntry[];
        }),
      ),
  );
  return results.flat().sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt));
}
