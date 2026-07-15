import type { SourceEntry, SourceDetail } from '../types';

export const entries: SourceEntry[] = [
  {
    id: 'gdrive-folder-reports',
    sourceId: 'gdrive',
    title: '보고서',
    kind: 'folder',
    modifiedAt: '2026-07-02T09:00:00.000Z',
    path: '/reports',
  },
  {
    id: 'gdrive-folder-contracts',
    sourceId: 'gdrive',
    title: '계약서',
    kind: 'folder',
    modifiedAt: '2026-06-28T11:00:00.000Z',
    path: '/contracts',
  },
  {
    id: 'gdrive-file-q2-report',
    sourceId: 'gdrive',
    title: '2026년 2분기 실적 보고서.pdf',
    kind: 'file',
    size: 2_457_600,
    modifiedAt: '2026-07-01T15:30:00.000Z',
    path: '/reports/2026년 2분기 실적 보고서.pdf',
  },
  {
    id: 'gdrive-file-marketing-plan',
    sourceId: 'gdrive',
    title: '마케팅 캠페인 기획서 Q3.pptx',
    kind: 'file',
    size: 5_120_000,
    modifiedAt: '2026-06-30T14:00:00.000Z',
    path: '/reports/마케팅 캠페인 기획서 Q3.pptx',
  },
  {
    id: 'gdrive-file-service-contract',
    sourceId: 'gdrive',
    title: '신규 서비스 계약서 2026-07.docx',
    kind: 'file',
    size: 384_000,
    modifiedAt: '2026-06-29T10:00:00.000Z',
    path: '/contracts/신규 서비스 계약서 2026-07.docx',
  },
  {
    id: 'gdrive-file-nda',
    sourceId: 'gdrive',
    title: 'NDA 체결서 (A사).pdf',
    kind: 'file',
    size: 256_000,
    modifiedAt: '2026-06-27T09:30:00.000Z',
    path: '/contracts/NDA 체결서 (A사).pdf',
  },
  {
    id: 'gdrive-file-security-audit',
    sourceId: 'gdrive',
    title: '보안 감사 보고서 2026-H1.pdf',
    kind: 'file',
    size: 1_048_576,
    modifiedAt: '2026-06-25T16:00:00.000Z',
    path: '/reports/보안 감사 보고서 2026-H1.pdf',
  },
  {
    id: 'gdrive-file-infra-migration',
    sourceId: 'gdrive',
    title: '인프라 마이그레이션 계획서.xlsx',
    kind: 'file',
    size: 768_000,
    modifiedAt: '2026-06-23T11:00:00.000Z',
    path: '/reports/인프라 마이그레이션 계획서.xlsx',
  },
  {
    id: 'gdrive-file-customer-feedback',
    sourceId: 'gdrive',
    title: '고객 피드백 분석 2026-06.docx',
    kind: 'file',
    size: 512_000,
    modifiedAt: '2026-06-20T13:30:00.000Z',
    path: '/reports/고객 피드백 분석 2026-06.docx',
  },
];

export const details: Record<string, SourceDetail> = {
  'gdrive-folder-reports': {
    entry: entries[0],
    externalUrl: '#',
  },
  'gdrive-folder-contracts': {
    entry: entries[1],
    externalUrl: '#',
  },
  'gdrive-file-q2-report': {
    entry: entries[2],
    externalUrl: '#',
  },
  'gdrive-file-marketing-plan': {
    entry: entries[3],
    externalUrl: '#',
  },
  'gdrive-file-service-contract': {
    entry: entries[4],
    externalUrl: '#',
  },
  'gdrive-file-nda': {
    entry: entries[5],
    externalUrl: '#',
  },
  'gdrive-file-security-audit': {
    entry: entries[6],
    externalUrl: '#',
  },
  'gdrive-file-infra-migration': {
    entry: entries[7],
    externalUrl: '#',
  },
  'gdrive-file-customer-feedback': {
    entry: entries[8],
    externalUrl: '#',
  },
};
