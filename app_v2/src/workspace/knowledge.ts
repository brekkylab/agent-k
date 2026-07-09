import { entries as knowledgeEntries } from './fixtures/knowledge';
import type { KnowledgeEvidenceRef, KnowledgeStatus, SourceEntry } from './types';

export interface KnowledgeSourceDocument {
  key: string;
  evidence: KnowledgeEvidenceRef;
  records: SourceEntry[];
  sourceEntry: SourceEntry;
}

function titleFromEvidence(label: string): string {
  const slashIndex = label.indexOf(' / ');
  return slashIndex >= 0 ? label.slice(slashIndex + 3) : label;
}

function sourceKind(sourceId: string): SourceEntry['kind'] {
  if (sourceId === 'notion') return 'page';
  if (sourceId === 'gmail' || sourceId === 'slack') return 'thread';
  if (sourceId === 'local' || sourceId === 'gdrive' || sourceId === 's3' || sourceId === 'dropbox') return 'file';
  return 'item';
}

export function sourceEntryFromEvidence(evidence: KnowledgeEvidenceRef): SourceEntry {
  const entry: SourceEntry = {
    id: evidence.entryId,
    sourceId: evidence.sourceId,
    title: titleFromEvidence(evidence.label),
    subtitle: evidence.label,
    kind: sourceKind(evidence.sourceId),
    modifiedAt: new Date().toISOString(),
  };
  if (entry.kind === 'file') {
    entry.path = evidence.entryId.startsWith('/') ? evidence.entryId : `/${evidence.entryId}`;
  }
  return entry;
}

export function evidenceForRecord(entry: SourceEntry): KnowledgeEvidenceRef[] {
  return entry.evidenceRefs ?? [];
}

export function getKnowledgeRecordsForSource(entry: SourceEntry): SourceEntry[] {
  return knowledgeEntries.filter((record) =>
    evidenceForRecord(record).some((evidence) =>
      evidence.sourceId === entry.sourceId &&
      (evidence.entryId === entry.id || evidence.entryId === entry.path),
    ),
  );
}

export function getKnowledgeSourceDocuments(records: SourceEntry[] = knowledgeEntries): KnowledgeSourceDocument[] {
  const grouped = new Map<string, KnowledgeSourceDocument>();

  for (const record of records) {
    for (const evidence of evidenceForRecord(record)) {
      const key = `${evidence.sourceId}:${evidence.entryId}`;
      const existing = grouped.get(key);
      if (existing) {
        existing.records.push(record);
        if (record.modifiedAt > existing.sourceEntry.modifiedAt) {
          existing.sourceEntry.modifiedAt = record.modifiedAt;
        }
        continue;
      }
      const sourceEntry = sourceEntryFromEvidence(evidence);
      sourceEntry.modifiedAt = record.modifiedAt;
      grouped.set(key, {
        key,
        evidence,
        records: [record],
        sourceEntry,
      });
    }
  }

  return [...grouped.values()].sort((a, b) => b.records[0]!.modifiedAt.localeCompare(a.records[0]!.modifiedAt));
}

export function statusCounts(records: SourceEntry[]): Record<KnowledgeStatus, number> {
  const base: Record<KnowledgeStatus, number> = { approved: 0, draft: 0, conflict: 0, stale: 0 };
  for (const record of records) {
    base[record.status ?? 'draft'] += 1;
  }
  return base;
}
