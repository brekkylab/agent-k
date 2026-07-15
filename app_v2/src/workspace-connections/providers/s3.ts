// Real S3 SourceProvider, backed by a workspace VFS mount served over the same
// WebDAV endpoint as local files. The backend (vfs/resource/s3.rs) maps object
// keys to paths under the mount prefix, so an S3 mount is just a plain file
// tree rooted at `/{prefix}` — this mirrors `localProvider` (which is rooted at
// `/`), only re-based onto the mount prefix.

import type { FileStat } from 'webdav';
import { listDirectory } from '@/api/workspace';
import type { MountResponse } from '@/api/mounts';
import type { SourceEntry, SourceProvider, SourceDetail, ListCtx } from '../types';

// Bare mount prefix ("/s3-prod" or "s3-prod/" → "s3-prod").
export function barePrefix(prefix: string): string {
  return prefix.replace(/^\/+/, '').replace(/\/+$/, '');
}

// Every entry carries the provider INSTANCE id (mount id) as sourceId, so the
// detail/list panels can re-resolve *which* S3 connection owns it via
// useProvider(entry.sourceId).
function statToEntry(stat: FileStat, instanceId: string): SourceEntry {
  const normPath = stat.filename.startsWith('/') ? stat.filename : '/' + stat.filename;
  return {
    id: normPath,
    sourceId: instanceId,
    title: stat.basename,
    kind: stat.type === 'directory' ? 'folder' : 'file',
    size: stat.type === 'directory' ? undefined : (stat.size ?? undefined),
    modifiedAt: new Date(stat.lastmod).toISOString(),
    path: normPath,
  };
}

export function makeS3Provider(mount: MountResponse): SourceProvider {
  const root = '/' + barePrefix(mount.prefix);
  const id = mount.id;

  return {
    id,
    type: 's3',
    label: mount.label ?? barePrefix(mount.prefix),
    nameKey: 'workspace.src.s3',
    category: 'files',
    kind: 'files',
    connected: true,
    attachable: false,
    count: null, // mount rows carry no count
    async list(ctx: ListCtx): Promise<SourceEntry[]> {
      // Entries carry full workspace paths (e.g. "/s3-prod/reports"), so a
      // navigation passes that path straight back as ctx.path; the first open
      // (no ctx.path) defaults to the mount root.
      const stats = await listDirectory(ctx.path ?? root);
      return stats.map((s) => statToEntry(s, id));
    },
    async recent(): Promise<SourceEntry[]> {
      const stats = await listDirectory(root);
      return stats
        .filter((s) => s.type !== 'directory')
        .map((s) => statToEntry(s, id))
        .sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt))
        .slice(0, 20);
    },
    async detail(entryId: string): Promise<SourceDetail> {
      // The id is the normalized webdav path — list its parent so nested
      // entries (e.g. "/s3-prod/reports/q2.pdf") resolve, not just root objects.
      const parent = entryId.slice(0, entryId.lastIndexOf('/')) || root;
      const stats = await listDirectory(parent);
      const stat = stats.find((s) => {
        const normPath = s.filename.startsWith('/') ? s.filename : '/' + s.filename;
        return normPath === entryId;
      });
      if (!stat) throw new Error(`s3: no entry for ${entryId}`);
      return { entry: statToEntry(stat, id) };
    },
  };
}
