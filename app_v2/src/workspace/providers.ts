import type { SourceEntry, SourceProvider } from './types';
import { localProvider } from './providers/local';
import { makeMockProvider } from './providers/mock';
import * as gdrive from './fixtures/gdrive';
import * as s3 from './fixtures/s3';
import * as confluence from './fixtures/confluence';
import * as jira from './fixtures/jira';
import * as gmail from './fixtures/gmail';
import * as slack from './fixtures/slack';

export const PROVIDERS: SourceProvider[] = [
  localProvider,
  makeMockProvider({ id: 'gdrive', nameKey: 'workspace.src.gdrive', category: 'files', kind: 'files', count: gdrive.entries.length }, gdrive),
  makeMockProvider({ id: 's3', nameKey: 'workspace.src.s3', category: 'files', kind: 'files', count: s3.entries.length }, s3),
  makeMockProvider({ id: 'confluence', nameKey: 'workspace.src.confluence', category: 'docs', kind: 'items', count: confluence.entries.length }, confluence),
  makeMockProvider({ id: 'jira', nameKey: 'workspace.src.jira', category: 'docs', kind: 'items', count: jira.entries.length }, jira),
  makeMockProvider({ id: 'gmail', nameKey: 'workspace.src.gmail', category: 'messages', kind: 'threads', count: gmail.entries.length }, gmail),
  makeMockProvider({ id: 'slack', nameKey: 'workspace.src.slack', category: 'messages', kind: 'threads', count: slack.entries.length }, slack),
];

export function getProvider(id: string): SourceProvider | undefined {
  return PROVIDERS.find((p) => p.id === id);
}

export async function allRecent(): Promise<SourceEntry[]> {
  const results = await Promise.all(
    PROVIDERS.map((p) => p.recent().catch((err) => { console.warn(`[workspace] ${p.id} recent() failed`, err); return [] as SourceEntry[]; }))
  );
  return results.flat().sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt));
}
