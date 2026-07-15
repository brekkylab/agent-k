import type { KnowledgeEvidenceRef, SourceDetail, SourceEntry } from '../types';

const q3Evidence: KnowledgeEvidenceRef[] = [
  {
    id: 'ev-q3-notion-strategy',
    sourceId: 'notion',
    entryId: 'notion-page-q3-product-strategy',
    label: 'Notion / Q3 Product Strategy',
    excerpt: 'Q3의 제품 전략은 모바일 응답성과 workspace reload 안정성을 우선순위로 둡니다.',
    usedFor: 'decision.summary',
  },
  {
    id: 'ev-q3-slack-product',
    sourceId: 'slack',
    entryId: 'slack-thread-q3-planning',
    label: 'Slack #product',
    excerpt: '이번 분기 핵심은 성능 개선과 모바일 대응입니다.',
    usedFor: 'supporting evidence',
  },
  {
    id: 'ev-q3-jira-dev-201',
    sourceId: 'jira',
    entryId: 'jira-issue-DEV-201',
    label: 'Jira DEV-201',
    excerpt: '결제 모듈 성능 개선 목표는 응답 시간 50% 단축입니다.',
    usedFor: 'metric target',
  },
];

const uploadEvidence: KnowledgeEvidenceRef[] = [
  {
    id: 'ev-upload-gmail-complaint',
    sourceId: 'gmail',
    entryId: 'gmail-thread-customer-complaint',
    label: 'Gmail / 서비스 오류 관련 불편 접수',
    excerpt: '파일 업로드 기능이 작동하지 않았다는 고객 불편 접수가 3건 있었습니다.',
    usedFor: 'fact.claim',
  },
  {
    id: 'ev-upload-slack-incident',
    sourceId: 'slack',
    entryId: 'slack-thread-incident-storage',
    label: 'Slack #incidents',
    excerpt: 'S3 연결 타임아웃 에러 발생. 고객 영향 3분 추정.',
    usedFor: 'root cause',
  },
];

const ndaEvidence: KnowledgeEvidenceRef[] = [
  {
    id: 'ev-nda-gmail-legal',
    sourceId: 'gmail',
    entryId: 'gmail-thread-legal-contract',
    label: 'Gmail / NDA 계약서 최종 검토 완료',
    excerpt: '기밀 유지 기간 5년에서 3년으로 수정 협의 완료.',
    usedFor: 'proposed contract field',
  },
  {
    id: 'ev-nda-shared-draft',
    sourceId: 'local',
    entryId: '/A사_NDA_draft.pdf',
    label: 'Shared Files / A사_NDA_draft.pdf',
    excerpt: '최신 draft에는 기밀 유지 기간이 5년으로 남아 있습니다.',
    usedFor: 'conflicting evidence',
  },
];

const groundingEvidence: KnowledgeEvidenceRef[] = [
  {
    id: 'ev-grounding-notion',
    sourceId: 'notion',
    entryId: 'notion-page-workspace-source-grounding',
    label: 'Notion / Workspace Source Grounding',
    excerpt: '답변은 source별 provenance chip을 가져야 합니다.',
    usedFor: 'open question',
  },
  {
    id: 'ev-grounding-confluence',
    sourceId: 'confluence',
    entryId: 'confluence-page-arch-decision',
    label: 'Confluence / 마이크로서비스 전환 결정 기록',
    excerpt: '구조화된 결정 기록은 변경 이유와 근거 문서를 함께 유지합니다.',
    usedFor: 'operating precedent',
  },
];

const customerEvidence: KnowledgeEvidenceRef[] = [
  {
    id: 'ev-customer-notion-objection',
    sourceId: 'notion',
    entryId: 'notion-page-objection-library',
    label: 'Notion / Objection Library',
    excerpt: 'Enterprise buyers ask for audit trail before connector rollout.',
    usedFor: 'customer insight',
  },
  {
    id: 'ev-customer-gmail-partner',
    sourceId: 'gmail',
    entryId: 'gmail-thread-partner-intro',
    label: 'Gmail / 파트너십 제안',
    excerpt: '워크플로우와 시너지가 있을 것 같아 파트너십을 제안드립니다.',
    usedFor: 'related opportunity',
  },
];

export const entries: SourceEntry[] = [
  {
    id: 'knowledge-decision-q3-mobile-performance',
    sourceId: 'knowledge',
    title: 'Q3 priority is mobile performance and reliability',
    subtitle: 'Decision · approved',
    kind: 'record',
    collection: 'Decisions',
    status: 'approved',
    confidence: 0.92,
    evidenceRefs: q3Evidence,
    modifiedAt: '2026-07-03T12:20:00.000Z',
  },
  {
    id: 'knowledge-fact-upload-incident',
    sourceId: 'knowledge',
    title: 'Upload failures on June 26 came from a storage timeout',
    subtitle: 'Fact · approved',
    kind: 'record',
    collection: 'Facts',
    status: 'approved',
    confidence: 0.88,
    evidenceRefs: uploadEvidence,
    modifiedAt: '2026-07-02T09:10:00.000Z',
  },
  {
    id: 'knowledge-contract-nda-retention',
    sourceId: 'knowledge',
    title: 'A사 NDA confidentiality period changed from 5 years to 3 years',
    subtitle: 'Contract · conflict',
    kind: 'record',
    collection: 'Contracts',
    status: 'conflict',
    confidence: 0.64,
    evidenceRefs: ndaEvidence,
    modifiedAt: '2026-07-01T16:40:00.000Z',
  },
  {
    id: 'knowledge-question-source-grounding',
    sourceId: 'knowledge',
    title: 'Which source types should be promoted into typed workspace records?',
    subtitle: 'Question · draft',
    kind: 'record',
    collection: 'Open Questions',
    status: 'draft',
    confidence: 0.51,
    evidenceRefs: groundingEvidence,
    modifiedAt: '2026-06-30T15:00:00.000Z',
  },
  {
    id: 'knowledge-customer-objection-library',
    sourceId: 'knowledge',
    title: 'Enterprise buyers ask for audit trail before connector rollout',
    subtitle: 'Customer · stale',
    kind: 'record',
    collection: 'Customers',
    status: 'stale',
    confidence: 0.73,
    evidenceRefs: customerEvidence,
    modifiedAt: '2026-06-28T10:45:00.000Z',
  },
];

export const details: Record<string, SourceDetail> = {
  'knowledge-decision-q3-mobile-performance': {
    entry: entries[0],
    bodyPreview:
      'Approved workspace decision for Q3 planning.\nMobile performance and reliability should be treated as the primary engineering priority. Product and engineering both mention mobile response time and reload resilience as the clearest near-term wedge.',
    externalUrl: '#',
  },
  'knowledge-fact-upload-incident': {
    entry: entries[1],
    bodyPreview:
      'Approved fact extracted from incident follow-up.\nThree upload complaints on 2026-06-26 match the storage timeout window mentioned in #incidents. The current state is resolved, but support follow-up should cite the storage timeout rather than a generic upload bug.',
    externalUrl: '#',
  },
  'knowledge-contract-nda-retention': {
    entry: entries[2],
    bodyPreview:
      'Needs review before answers should rely on it.\nThe legal email says the confidentiality period was negotiated from 5 years to 3 years, but the latest shared draft still appears to mention 5 years. Resolve the contract source before marking this approved.',
    externalUrl: '#',
  },
  'knowledge-question-source-grounding': {
    entry: entries[3],
    bodyPreview:
      'Draft question for workspace shaping.\nThe workspace already has files, pages, tickets, and messages. The unresolved product question is which source moments deserve promotion into structured records instead of remaining raw searchable context.',
    externalUrl: '#',
  },
  'knowledge-customer-objection-library': {
    entry: entries[4],
    bodyPreview:
      'Stale customer insight.\nEarlier customer notes suggest enterprise buyers need a visible audit trail before connector rollout. This should be refreshed with newer sales or research notes before being used in a launch plan.',
    externalUrl: '#',
  },
};
