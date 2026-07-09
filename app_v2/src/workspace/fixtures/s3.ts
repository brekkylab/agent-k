import type { SourceEntry, SourceDetail } from '../types';

export const entries: SourceEntry[] = [
  {
    id: 's3-folder-releases',
    sourceId: 's3',
    title: 'releases',
    kind: 'folder',
    modifiedAt: '2026-07-02T08:00:00.000Z',
    path: '/releases',
  },
  {
    id: 's3-folder-backups',
    sourceId: 's3',
    title: 'backups',
    kind: 'folder',
    modifiedAt: '2026-06-30T06:00:00.000Z',
    path: '/backups',
  },
  {
    id: 's3-file-deploy-note-v230',
    sourceId: 's3',
    title: 'v2.3.0 배포 노트.md',
    kind: 'file',
    size: 32_768,
    modifiedAt: '2026-07-01T12:00:00.000Z',
    path: '/releases/v2.3.0 배포 노트.md',
  },
  {
    id: 's3-file-deploy-note-v221',
    sourceId: 's3',
    title: 'v2.2.1 핫픽스 배포 노트.md',
    kind: 'file',
    size: 16_384,
    modifiedAt: '2026-06-25T10:30:00.000Z',
    path: '/releases/v2.2.1 핫픽스 배포 노트.md',
  },
  {
    id: 's3-file-db-backup-20260702',
    sourceId: 's3',
    title: 'db-backup-2026-07-02.tar.gz',
    kind: 'file',
    size: 524_288_000,
    modifiedAt: '2026-07-02T03:00:00.000Z',
    path: '/backups/db-backup-2026-07-02.tar.gz',
  },
  {
    id: 's3-file-db-backup-20260701',
    sourceId: 's3',
    title: 'db-backup-2026-07-01.tar.gz',
    kind: 'file',
    size: 521_142_272,
    modifiedAt: '2026-07-01T03:00:00.000Z',
    path: '/backups/db-backup-2026-07-01.tar.gz',
  },
  {
    id: 's3-file-assets-bundle',
    sourceId: 's3',
    title: 'assets-bundle-v2.3.0.zip',
    kind: 'file',
    size: 102_400_000,
    modifiedAt: '2026-07-01T11:45:00.000Z',
    path: '/releases/assets-bundle-v2.3.0.zip',
  },
  {
    id: 's3-file-log-archive-june',
    sourceId: 's3',
    title: 'logs-2026-06.tar.gz',
    kind: 'file',
    size: 209_715_200,
    modifiedAt: '2026-06-30T23:59:00.000Z',
    path: '/backups/logs-2026-06.tar.gz',
  },
  {
    id: 's3-file-ml-model-v4',
    sourceId: 's3',
    title: 'recommendation-model-v4.bin',
    kind: 'file',
    size: 1_073_741_824,
    modifiedAt: '2026-06-22T09:00:00.000Z',
    path: '/releases/recommendation-model-v4.bin',
  },
  {
    id: 's3-file-design-system-export',
    sourceId: 's3',
    title: '디자인 시스템 에셋 v1.4.zip',
    kind: 'file',
    size: 78_643_200,
    modifiedAt: '2026-06-19T14:00:00.000Z',
    path: '/releases/디자인 시스템 에셋 v1.4.zip',
  },
];

export const details: Record<string, SourceDetail> = {
  's3-folder-releases': {
    entry: entries[0],
    externalUrl: '#',
  },
  's3-folder-backups': {
    entry: entries[1],
    externalUrl: '#',
  },
  's3-file-deploy-note-v230': {
    entry: entries[2],
    externalUrl: '#',
  },
  's3-file-deploy-note-v221': {
    entry: entries[3],
    externalUrl: '#',
  },
  's3-file-db-backup-20260702': {
    entry: entries[4],
    externalUrl: '#',
  },
  's3-file-db-backup-20260701': {
    entry: entries[5],
    externalUrl: '#',
  },
  's3-file-assets-bundle': {
    entry: entries[6],
    externalUrl: '#',
  },
  's3-file-log-archive-june': {
    entry: entries[7],
    externalUrl: '#',
  },
  's3-file-ml-model-v4': {
    entry: entries[8],
    externalUrl: '#',
  },
  's3-file-design-system-export': {
    entry: entries[9],
    externalUrl: '#',
  },
};
