export type SourceCategory = 'files' | 'docs' | 'messages';
export type SourceKind = 'files' | 'items' | 'threads';

export interface SourceEntry {
  id: string;               // provider-unique
  sourceId: string;         // provider id
  title: string;
  subtitle?: string;        // sender/space/status line for items & threads
  kind: 'file' | 'folder' | 'item' | 'thread';
  size?: number;            // bytes, files only
  modifiedAt: string;       // ISO
  path?: string;            // files: webdav-relative path e.g. "/reports/q2.pdf"
}

export interface SourceDetail {
  entry: SourceEntry;
  bodyPreview?: string;     // items/threads: text body; threads render as bubbles (speaker + text lines separated by \n)
  externalUrl?: string;     // mock sources: '원본 열기' target (may be '#')
}

export interface ListCtx { path?: string }   // files archetype: current folder ('' = root)

export interface SourceProvider {
  id: 'local' | 'gdrive' | 's3' | 'confluence' | 'jira' | 'gmail' | 'slack';
  nameKey: string;          // i18n key in `files` ns, e.g. 'workspace.src.local'
  category: SourceCategory;
  kind: SourceKind;
  connected: boolean;       // all true in v1 (mocks demo as connected)
  attachable: boolean;      // local only
  count: number | null;     // rail badge; null → '—'
  list(ctx: ListCtx): Promise<SourceEntry[]>;
  recent(): Promise<SourceEntry[]>;     // ≤20, newest first
  detail(id: string): Promise<SourceDetail>;
}
