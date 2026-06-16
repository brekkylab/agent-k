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

/** Max files attachable to one message. Mirrors the backend's server-side
 *  ceiling in `validate_attachments`; an import that would exceed it is rejected
 *  wholesale (frontend) and 400'd (backend). */
export const MAX_ATTACHMENTS = 30;

export interface ExpandedFile { globalPath: string; filename: string; }

/**
 * Expand dirent global paths to the files they cover: a folder yields its
 * descendant files (recursive), a file yields itself. Hidden names (.keep and
 * other dotfiles) are skipped and results are deduped by global path.
 * `entries` must be the recursive listing the paths come from.
 */
export function expandDirentPaths(entries: BackendDirent[], globalPaths: Iterable<string>): ExpandedFile[] {
  const out: ExpandedFile[] = [];
  const seen = new Set<string>();
  const add = (gp: string) => {
    if (seen.has(gp)) return;
    seen.add(gp);
    out.push({ globalPath: gp, filename: gp.split('/').filter(Boolean).pop() ?? gp });
  };
  for (const p of globalPaths) {
    const entry = entries.find((e) => e.path === p);
    if (entry?.kind === 'dir') {
      const prefix = `${p}/`;
      for (const e of entries) {
        if (e.kind === 'file' && e.path.startsWith(prefix) && !isHiddenName(nameOf(e))) add(e.path);
      }
    } else {
      add(p);
    }
  }
  return out;
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
  if (/\.(csv|tsv|xlsx|xls|ods|numbers)$/.test(lower)) return 'sheet';
  if (/\.(png|jpe?g|gif|webp|svg|bmp|heic|heif|avif|tiff?|ico|raw|cr2|nef|arw)$/.test(lower)) return 'image';
  if (/\.(md|txt|doc|docx|markdown|rtf|odt|hwp|hwpx|pages|pptx?|odp|key|epub)$/.test(lower)) return 'doc';
  if (/\.(js|mjs|cjs|ts|tsx|jsx|json|jsonc|html|htm|css|scss|sass|less|py|rb|rs|go|java|kt|kts|c|cc|cpp|h|hpp|cs|fs|fsx|swift|m|sh|bash|zsh|fish|ps1|bat|cmd|yml|yaml|toml|xml|sql|graphql|gql|lua|php|r|scala|clj|cljs|ex|exs|erl|hrl|hs|ml|mli|nim|zig|dart|vue|svelte|elm|tf|tfvars|bicep|proto|gradle|groovy|cmake|make|dockerfile|ini|env|cfg|conf)$/.test(lower)) return 'code';
  if (/\.(zip|tar|gz|tgz|bz2|xz|lz4|zst|rar|7z|cab|apk|ipa|deb|rpm|dmg|pkg|iso)$/.test(lower)) return 'archive';
  if (/\.(mp4|mov|avi|mkv|webm|m4v|wmv|flv|3gp|ts|mts|m2ts|vob|ogv|rmvb)$/.test(lower)) return 'video';
  if (/\.(mp3|wav|flac|ogg|m4a|aac|weba|opus|mid|midi|aiff|au|wma|amr|ape|dsf)$/.test(lower)) return 'audio';
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

// ── Preview kind resolution ────────────────────────────────────────
// 이 resolver는 categorise()(색상/아이콘용 분류)와 의도적으로 다른 taxonomy다:
//   - categorise는 .html/.htm을 'code'로, .csv/.tsv를 'sheet'로, .ini/.env/.cfg/.conf를
//     'code'로 묶지만, 미리보기는 html=라이브 렌더, csv/tsv=표(table), ini/env/cfg/conf=평문 text로 분기한다.
//   - 따라서 categorise의 정규식을 재사용하지 않고 독립 정의한다(공유하면 잘못된 결합이 된다).
//     "preview kind"와 "badge color"는 서로 다른 관심사다.
export type PreviewKind = 'pdf' | 'image' | 'html' | 'markdown' | 'code' | 'table' | 'text' | 'unsupported';

const CODE_EXT =
  /\.(js|mjs|cjs|ts|tsx|jsx|json|jsonc|css|scss|sass|less|py|rb|rs|go|java|kt|kts|c|cc|cpp|h|hpp|cs|fs|fsx|swift|m|sh|bash|zsh|fish|ps1|bat|cmd|yml|yaml|toml|xml|sql|graphql|gql|lua|php|r|scala|clj|cljs|ex|exs|erl|hrl|hs|ml|mli|nim|zig|dart|vue|svelte|elm|tf|tfvars|bicep|proto|gradle|groovy|cmake|make|dockerfile)$/;

export function resolvePreviewKind(filename: string): PreviewKind {
  const lower = filename.toLowerCase();
  if (lower.endsWith('.pdf')) return 'pdf';
  if (/\.(png|jpe?g|gif|webp|svg|bmp|avif)$/.test(lower)) return 'image';
  if (/\.html?$/.test(lower)) return 'html';        // MUST precede code
  if (/\.(md|markdown)$/.test(lower)) return 'markdown';
  if (CODE_EXT.test(lower)) return 'code';
  if (/\.(csv|tsv)$/.test(lower)) return 'table';
  if (/\.(txt|log|env|ini|cfg|conf)$/.test(lower)) return 'text';
  return 'unsupported';
}

// highlight.js hint for code preview; '' lets rehype-highlight auto-detect.
export function previewCodeLang(filename: string): string {
  const ext = filename.toLowerCase().split('.').pop() ?? '';
  return ext === filename.toLowerCase() ? '' : ext;
}
