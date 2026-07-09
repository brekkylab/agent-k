export type SourceCategory = 'files' | 'docs' | 'messages' | 'knowledge';
export type SourceKind = 'files' | 'items' | 'threads' | 'pages' | 'records';
export type KnowledgeStatus = 'approved' | 'draft' | 'conflict' | 'stale';

export interface KnowledgeEvidenceRef {
  id: string;
  sourceId: string;
  entryId: string;
  label: string;
  excerpt: string;
  usedFor: string;
}

export interface SourceEntry {
  id: string;               // provider-unique
  sourceId: string;         // provider id
  title: string;
  subtitle?: string;        // sender/space/status line for items & threads
  kind: 'file' | 'folder' | 'item' | 'thread' | 'page' | 'record';
  size?: number;            // bytes, files only
  modifiedAt: string;       // ISO
  path?: string;            // files: webdav-relative path e.g. "/reports/q2.pdf"
  parentId?: string | null;  // page tree providers: null/root vs parent page id
  emoji?: string;            // page tree providers: page icon
  collection?: string;       // knowledge records: Facts, Decisions, Customers, etc.
  status?: KnowledgeStatus;  // knowledge records: curation state
  confidence?: number;       // knowledge records: 0..1 confidence signal
  evidenceRefs?: KnowledgeEvidenceRef[]; // knowledge records: structured provenance
}

export interface SourceDetail {
  entry: SourceEntry;
  bodyPreview?: string;     // items/threads: text body; threads render as bubbles (speaker + text lines separated by \n)
  externalUrl?: string;     // mock sources: '원본 열기' target (may be '#')
}

export interface ListCtx { path?: string }   // files archetype: current folder ('' = root)

export interface SourceProvider {
  id: 'local' | 'gdrive' | 's3' | 'dropbox' | 'figma' | 'confluence' | 'notion' | 'jira' | 'github' | 'linear' | 'gmail' | 'slack' | 'knowledge';
  nameKey: string;          // i18n key in `files` ns, e.g. 'workspace.src.local'
  category: SourceCategory;
  kind: SourceKind;
  connected: boolean;       // false means visible in the catalog but not browseable until mock-connected
  attachable: boolean;      // local only
  count: number | null;     // rail badge; null → '—'
  list(ctx: ListCtx): Promise<SourceEntry[]>;
  recent(): Promise<SourceEntry[]>;     // ≤20, newest first
  detail(id: string): Promise<SourceDetail>;
}
