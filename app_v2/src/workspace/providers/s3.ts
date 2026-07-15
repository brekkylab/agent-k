// Real S3 SourceProvider, backed by a workspace VFS mount served over the same
// WebDAV endpoint as local files. The backend (vfs/resource/s3.rs) maps object
// keys to paths under the mount prefix, so an S3 mount is just a plain file
// tree rooted at `/{prefix}` — this mirrors `localProvider` (which is rooted at
// `/`), only re-based onto the mount prefix.

import type { FileStat } from 'webdav';
import { listDirectory } from '@/api/workspace';
import type { SourceEntry, SourceProvider, SourceDetail, ListCtx } from '../types';

// Normalize the stored mount prefix ("/s3-prod" or "s3-prod/") to a bare,
// leading-slash root ("/s3-prod").
function rootOf(mountPrefix: string): string {
  return '/' + mountPrefix.replace(/^\/+/, '').replace(/\/+$/, '');
}

function statToEntry(stat: FileStat): SourceEntry {
  const normPath = stat.filename.startsWith('/') ? stat.filename : '/' + stat.filename;
  return {
    id: normPath,
    sourceId: 's3',
    title: stat.basename,
    kind: stat.type === 'directory' ? 'folder' : 'file',
    size: stat.type === 'directory' ? undefined : (stat.size ?? undefined),
    modifiedAt: new Date(stat.lastmod).toISOString(),
    path: normPath,
  };
}

export function makeS3Provider(mountPrefix: string): SourceProvider {
  const root = rootOf(mountPrefix);

  return {
    id: 's3',
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
      return stats.map(statToEntry);
    },
    async recent(): Promise<SourceEntry[]> {
      const stats = await listDirectory(root);
      return stats
        .filter((s) => s.type !== 'directory')
        .map(statToEntry)
        .sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt))
        .slice(0, 20);
    },
    async detail(id: string): Promise<SourceDetail> {
      // The id is the normalized webdav path — list its parent so nested
      // entries (e.g. "/s3-prod/reports/q2.pdf") resolve, not just root objects.
      const parent = id.slice(0, id.lastIndexOf('/')) || root;
      const stats = await listDirectory(parent);
      const stat = stats.find((s) => {
        const normPath = s.filename.startsWith('/') ? s.filename : '/' + s.filename;
        return normPath === id;
      });
      if (!stat) throw new Error(`s3: no entry for ${id}`);
      return { entry: statToEntry(stat) };
    },
  };
}
