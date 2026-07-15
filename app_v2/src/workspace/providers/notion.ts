// Real Notion SourceProvider, backed by a workspace VFS mount served over the
// same WebDAV endpoint as local files. The backend (vfs/resource/notion.rs)
// exposes the mount as `{prefix}/pages/<title>__<id>/page.json` (+ nested child
// page dirs, recursively).
//
// The page tree is built from a DIRECTORY WALK ONLY — no page.json GETs — because
// reading a page.json triggers a full Notion render on the backend (get_page +
// recursive block tree), whereas a directory listing carries name + mtime for
// free. id / title / parentId / modifiedAt all come from the dir entries; the
// single render is deferred to detail() on click.

import { listDirectory, getFileText } from '@/api/workspace';
import type { SourceEntry, SourceDetail, SourceProvider } from '../types';

// The `<title>__<id>` page-dir naming; the id is everything after the LAST `__`
// (sanitize_name folds runs of `_`, so the title part never contains `__`).
function splitPageDir(basename: string): { id: string; title: string } | null {
  const sep = basename.lastIndexOf('__');
  if (sep === -1) return null;
  const id = basename.slice(sep + 2);
  if (!id) return null;
  const rawTitle = basename.slice(0, sep);
  // Approximate the display title from the sanitized dir name (underscores were
  // spaces / folded specials). Exact title comes from detail().
  const title = rawTitle ? rawTitle.replace(/_/g, ' ') : 'Untitled';
  return { id, title };
}

// Normalize the stored mount prefix ("/notion") to a bare segment ("notion").
function barePrefix(prefix: string): string {
  return prefix.replace(/^\/+/, '').replace(/\/+$/, '');
}

// The normalized page.json shape emitted by backend `normalize_page`.
interface NotionPageJson {
  page_id: string;
  title: string;
  url: string;
  markdown: string;
  parent_type: string;
  parent_id: string;
}

const MAX_DEPTH = 20; // defensive cap on page nesting recursion

export function makeNotionProvider(mountPrefix: string): SourceProvider {
  const prefix = barePrefix(mountPrefix);
  const pagesRoot = `/${prefix}/pages`;

  async function walk(
    dirPath: string,
    parentId: string | null,
    depth: number,
    out: SourceEntry[],
  ): Promise<void> {
    if (depth > MAX_DEPTH) return;
    const stats = await listDirectory(dirPath);
    for (const stat of stats) {
      // Only page directories are pages; `page.json` (a file) is skipped.
      if (stat.type !== 'directory') continue;
      const parsed = splitPageDir(stat.basename);
      if (!parsed) continue;
      out.push({
        id: parsed.id,
        sourceId: 'notion',
        title: parsed.title,
        kind: 'page',
        modifiedAt: new Date(stat.lastmod).toISOString(),
        parentId,
        emoji: '📄',
      });
      await walk(`${dirPath}/${stat.basename}`, parsed.id, depth + 1, out);
    }
  }

  async function listAll(): Promise<SourceEntry[]> {
    const out: SourceEntry[] = [];
    await walk(pagesRoot, null, 0, out);
    return out;
  }

  return {
    id: 'notion',
    nameKey: 'workspace.src.notion',
    category: 'docs',
    kind: 'pages',
    connected: true,
    attachable: false,
    count: null, // mount rows carry no count
    list: () => listAll(),
    recent: async () =>
      (await listAll())
        .sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt))
        .slice(0, 20),
    detail: async (id: string): Promise<SourceDetail> => {
      // read_bytes resolves the page purely by the id after the last `__`, so a
      // synthetic flat path works for any page regardless of nesting.
      const text = await getFileText(`/${prefix}/pages/x__${id}/page.json`);
      const page = JSON.parse(text) as NotionPageJson;
      return {
        entry: {
          id,
          sourceId: 'notion',
          title: page.title || 'Untitled',
          kind: 'page',
          modifiedAt: '',
          parentId: page.parent_type === 'workspace' || !page.parent_id ? null : page.parent_id,
          emoji: '📄',
        },
        bodyPreview: page.markdown,
        externalUrl: page.url || undefined,
      };
    },
  };
}
