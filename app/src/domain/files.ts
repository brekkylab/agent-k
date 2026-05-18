import type { BackendDirent } from '@/api/backend-types';
import type { FileAsset, ProjectId } from './types';

export type FolderInfo = { key: string; label: string; count: number };

// Legacy flat-folder helpers retained for callers outside the Files page.
export function getProjectFolders(files: FileAsset[], projectId: ProjectId): FolderInfo[] {
  const counts = new Map<string, number>();
  files.filter((file) => file.projectId === projectId).forEach((file) => counts.set(folderOf(file), (counts.get(folderOf(file)) ?? 0) + 1));
  if (!counts.size) counts.set('General', 0);
  return [...counts.entries()].map(([key, count]) => ({ key, label: key, count }));
}

export function folderOf(file: FileAsset): string {
  const parts = file.path.split('/').filter(Boolean);
  return parts[1] ?? 'General';
}

// ── Tree-aware helpers (Files page) ────────────────────────────────

export function isHiddenName(name: string): boolean {
  return name.startsWith('.');
}

export function nameOf(entry: BackendDirent): string {
  const parts = entry.path.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? entry.path;
}

// Returns entries that live one level directly under `pathSegments`.
// Excludes the directory's own row and dotfiles (.keep etc.).
export interface DirectChildren {
  folders: BackendDirent[];
  files: BackendDirent[];
}
export function listDirectChildren(entries: BackendDirent[], pathSegments: string[]): DirectChildren {
  const prefix = pathSegments.join('/');
  const prefixWithSlash = prefix ? `${prefix}/` : '';
  const folders: BackendDirent[] = [];
  const files: BackendDirent[] = [];
  for (const entry of entries) {
    if (entry.path === prefix) continue;
    if (prefix && !entry.path.startsWith(prefixWithSlash)) continue;
    const rel = prefix ? entry.path.slice(prefixWithSlash.length) : entry.path;
    if (!rel || rel.includes('/') || isHiddenName(rel)) continue;
    if (entry.kind === 'dir') folders.push(entry);
    else files.push(entry);
  }
  const byPath = (a: BackendDirent, b: BackendDirent) => a.path.localeCompare(b.path);
  folders.sort(byPath);
  files.sort(byPath);
  return { folders, files };
}

// Visible file count under a directory (recursive). Dotfiles excluded.
export function countDescendants(entries: BackendDirent[], pathSegments: string[]): number {
  const prefix = `${pathSegments.join('/')}/`;
  let n = 0;
  for (const entry of entries) {
    if (entry.kind !== 'file') continue;
    if (!entry.path.startsWith(prefix)) continue;
    const tail = entry.path.slice(prefix.length);
    if (tail.split('/').some(isHiddenName)) continue;
    n += 1;
  }
  return n;
}

// Build a nested tree from the flat dirent list, folders only.
// Used by the sidebar tree (files don't surface there).
export interface FolderNode {
  name: string;
  path: string;            // backend path (no projectName prefix)
  segments: string[];      // path split
  children: FolderNode[];
}
export function buildFolderTree(entries: BackendDirent[]): FolderNode[] {
  const dirs = entries
    .filter((e) => e.kind === 'dir')
    .map((e) => e.path)
    .filter((p) => !p.split('/').some(isHiddenName))
    .sort((a, b) => a.localeCompare(b));

  const root: FolderNode[] = [];
  const byPath = new Map<string, FolderNode>();

  for (const path of dirs) {
    const segments = path.split('/').filter(Boolean);
    const name = segments[segments.length - 1] ?? path;
    const node: FolderNode = { name, path, segments, children: [] };
    byPath.set(path, node);
    if (segments.length === 1) {
      root.push(node);
    } else {
      const parentPath = segments.slice(0, -1).join('/');
      const parent = byPath.get(parentPath);
      if (parent) parent.children.push(node);
      else root.push(node); // orphan — surface at root so it's reachable
    }
  }
  return root;
}

// All ancestor segments of currentPath (for auto-expanding sidebar tree).
export function ancestorPaths(currentPath: string[]): string[] {
  const out: string[] = [];
  for (let i = 1; i <= currentPath.length; i += 1) {
    out.push(currentPath.slice(0, i).join('/'));
  }
  return out;
}

// Internal — categorise by extension. Both fileTypeClass and fileTypeIcon
// derive from this single source so colour and icon never drift apart.
type FileCategory = 'pdf' | 'sheet' | 'image' | 'doc' | 'code' | 'archive' | 'video' | 'audio' | 'other';

function categorise(name: string): FileCategory {
  const lower = name.toLowerCase();
  if (lower.endsWith('.pdf')) return 'pdf';
  if (/\.(csv|tsv|xlsx|xls)$/.test(lower)) return 'sheet';
  if (/\.(png|jpe?g|gif|webp|svg|bmp|heic|avif)$/.test(lower)) return 'image';
  if (/\.(md|txt|doc|docx|markdown|rtf|odt)$/.test(lower)) return 'doc';
  if (/\.(js|mjs|cjs|ts|tsx|jsx|json|html|css|scss|py|rb|rs|go|java|c|cc|cpp|h|hpp|sh|bash|zsh|yml|yaml|toml|xml|sql)$/.test(lower)) return 'code';
  if (/\.(zip|tar|gz|tgz|bz2|rar|7z)$/.test(lower)) return 'archive';
  if (/\.(mp4|mov|avi|mkv|webm|m4v)$/.test(lower)) return 'video';
  if (/\.(mp3|wav|flac|ogg|m4a|aac)$/.test(lower)) return 'audio';
  return 'other';
}

// Maps a filename to a design-system colour class.
export function fileTypeClass(name: string): string {
  switch (categorise(name)) {
    case 'pdf':     return 'cw-file-pdf';
    case 'sheet':   return 'cw-file-sheet';
    case 'image':   return 'cw-file-image';
    case 'doc':     return 'cw-file-doc';
    case 'code':    return 'cw-file-code';
    case 'archive': return 'cw-file-archive';
    case 'video':   return 'cw-file-video';
    case 'audio':   return 'cw-file-audio';
    default:        return 'cw-file-file';
  }
}

// Maps a filename to an Icon name (Lucide file family).
export function fileTypeIcon(name: string): 'file' | 'file-text' | 'sheet' | 'image' | 'file-pdf' | 'file-code' | 'file-archive' | 'file-video' | 'file-audio' {
  switch (categorise(name)) {
    case 'pdf':     return 'file-pdf';
    case 'sheet':   return 'sheet';
    case 'image':   return 'image';
    case 'doc':     return 'file-text';
    case 'code':    return 'file-code';
    case 'archive': return 'file-archive';
    case 'video':   return 'file-video';
    case 'audio':   return 'file-audio';
    default:        return 'file';
  }
}
