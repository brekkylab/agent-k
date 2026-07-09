// Workspace candidate C — Cultivation Canvas.
// Self-contained mock data for material -> evidence -> typed collection flows.

export type CcCardKind =
  | 'material'
  | 'clip'
  | 'record'
  | 'proposal'
  | 'gap'
  | 'conflict'
  | 'aiBubble'
  | 'sticky'
  | 'chart';

export type CcCardSource = 'drive' | 'gmail' | 'slack' | 'session' | 'upload';

export interface CcCard {
  id: string;
  kind: CcCardKind;
  source: CcCardSource;
  icon: string;
  title: string;
  origin: string;
  body: string;
  placed: boolean;
  x: number;
  y: number;
  width?: number;
  targetCollectionId?: string;
  targetFieldKey?: string;
  analyzing?: boolean;
  chartValues?: number[];
}

export interface CcCollectionField {
  key: string;
  label: string;
}

export interface CcCollectionRecord {
  id: string;
  title: string;
  status: 'approved' | 'draft';
  values: Record<string, string>;
  provenance: Record<string, { sourceTitle: string; quote: string }>;
  gaps?: Record<string, string>;
}

export interface CcCollection {
  id: string;
  icon: string;
  name: string;
  type: 'index' | 'typed';
  description: string;
  fields: CcCollectionField[];
  records: CcCollectionRecord[];
  docCount?: number;
}

export type CcLinkKind = 'derivedFrom' | 'supportsField' | 'conflictsWith' | 'belongsTo';

export interface CcLink {
  id: string;
  kind: CcLinkKind;
  from: string;
  to: string;
  label: string;
}

export interface CcConflict {
  id: string;
  fieldLabel: string;
  latestSource: string;
  olderSource: string;
  latestQuote: string;
  olderQuote: string;
}

export const INITIAL_CARDS: CcCard[] = [
  {
    id: 'quote-mail',
    kind: 'material',
    source: 'gmail',
    icon: '✉️',
    title: 'A클라우드 최종 견적 메일',
    origin: 'Gmail · 2026-07-01',
    placed: true,
    x: 54,
    y: 156,
    width: 288,
    body:
      '안녕하세요, A클라우드 영업팀입니다.\n\n' +
      '최종 견적은 월 340만 원이며, 월간 트래픽 10TB 초과분은 GB당 120원으로 산정됩니다.\n' +
      '3년 약정 시 15% 할인 적용 가능하며, 계약서 초안 검토 후 회신 부탁드립니다.',
  },
  {
    id: 'contract-draft',
    kind: 'material',
    source: 'drive',
    icon: '📋',
    title: '신규 서비스 계약서 2026-07.docx',
    origin: 'Google Drive · 9일 전',
    placed: true,
    x: 358,
    y: 156,
    width: 252,
    targetCollectionId: 'contracts',
    targetFieldKey: 'risk',
    body:
      '서비스 공급 계약서 초안\n\n' +
      '제9조 (손해배상 상한)\n' +
      '공급자의 손해배상 책임 총액은 최근 3개월 대금의 100분의 200을 상한으로 한다.\n\n' +
      '제12조 (데이터 반환)\n' +
      '계약 종료 후 30일 이내 요청 시 백업 데이터를 반환한다.',
  },
  {
    id: 'infra-chart',
    kind: 'chart',
    source: 'drive',
    icon: '📊',
    title: '월 인프라 비용 추이',
    origin: 'Google Sheets · 5일 전',
    placed: true,
    x: 74,
    y: 390,
    width: 268,
    body: '4월 280만 · 5월 295만 · 6월 310만 · 7월 예상 340만',
    chartValues: [280, 295, 310, 340],
  },
  {
    id: 'meeting-note',
    kind: 'sticky',
    source: 'session',
    icon: '📌',
    title: '9조 수정 요청',
    origin: '이번 세션 메모',
    placed: true,
    x: 378,
    y: 410,
    width: 220,
    body: '손해배상 상한 200% -> 100% 또는 150%로 하향 요청. 법무 검토 후 B사에 수정안 송부.',
  },
  {
    id: 'legal-review',
    kind: 'material',
    source: 'drive',
    icon: '⚖️',
    title: '법무팀 검토 의견서',
    origin: 'Google Drive · 3일 전',
    placed: false,
    x: 0,
    y: 0,
    width: 340,
    targetCollectionId: 'contracts',
    targetFieldKey: 'risk',
    body:
      '법무팀 인프라 계약 검토 의견\n\n' +
      '9조 손해배상 상한 200%는 표준 계약 대비 높은 편이므로 100% 또는 150%로 하향 요청 필요.\n' +
      '12조 데이터 반환 기간은 30일에서 60일로 연장 요청.\n' +
      '중도 해지 위약금 2개월치는 업계 통상 수준으로 확인됨.',
  },
  {
    id: 'traffic-data',
    kind: 'material',
    source: 'drive',
    icon: '📈',
    title: '트래픽 추이 데이터 Q2',
    origin: 'Google Sheets · 5일 전',
    placed: false,
    x: 0,
    y: 0,
    width: 300,
    body:
      '2분기 트래픽 집계\n\n' +
      '월 평균 트래픽은 7.8TB이며 전분기 대비 31% 증가했다. 6월 피크는 11.2TB로 초과 과금이 발생했다.',
  },
  {
    id: 'session-artifact',
    kind: 'material',
    source: 'session',
    icon: '🧮',
    title: '세션 산출물 · 비용 시뮬레이션',
    origin: '이번 세션',
    placed: false,
    x: 0,
    y: 0,
    width: 300,
    body:
      'A클라우드 초과 과금 시뮬레이션\n\n' +
      '10TB 초과분 1.2TB x GB당 120원 = 월 14.4만 원. 3분기 피크 기준 36만~60만 원 범위.',
  },
];

export const INITIAL_COLLECTIONS: CcCollection[] = [
  {
    id: 'knowledge-index',
    icon: '🔎',
    name: 'Knowledge Index #0',
    type: 'index',
    description: '비정형 자료가 먼저 들어오는 기본 검색 컬렉션',
    docCount: 8,
    fields: [],
    records: [],
  },
  {
    id: 'crm',
    icon: '📇',
    name: 'CRM',
    type: 'typed',
    description: '고객/딜 상태를 질의 가능한 근거로 유지',
    fields: [
      { key: 'name', label: '고객/딜' },
      { key: 'stage', label: '단계' },
      { key: 'lastTouch', label: '마지막 접점' },
      { key: 'next', label: '다음 액션' },
    ],
    records: [
      {
        id: 'crm-b',
        title: 'B사 — 서비스 공급',
        status: 'approved',
        values: {
          name: 'B사 — 서비스 공급',
          stage: '계약 협상',
          lastTouch: '6/30 내부 회의',
          next: '9조 수정안 송부',
        },
        provenance: {
          stage: {
            sourceTitle: '회의록 6/30',
            quote: '계약서 9조 수정안 법무팀 송부 합의.',
          },
          next: {
            sourceTitle: '이번 세션 메모',
            quote: '법무 검토 후 B사에 수정안 송부.',
          },
        },
      },
    ],
  },
  {
    id: 'contracts',
    icon: '📑',
    name: 'Contracts',
    type: 'typed',
    description: '계약 조항을 field/provenance 단위로 가꾸는 컬렉션',
    fields: [
      { key: 'party', label: '상대방' },
      { key: 'term', label: '기간' },
      { key: 'risk', label: '주의 조항' },
      { key: 'status', label: '상태' },
    ],
    records: [
      {
        id: 'contract-b',
        title: 'B사 인프라 계약',
        status: 'draft',
        values: {
          party: 'B사',
          term: '12개월',
          risk: '검토 필요',
          status: '협상 중',
        },
        provenance: {
          term: {
            sourceTitle: '신규 서비스 계약서 2026-07.docx',
            quote: '계약 기간 12개월',
          },
        },
        gaps: {
          risk: '법무 검토 의견 필요',
        },
      },
    ],
  },
  {
    id: 'meetings',
    icon: '🤝',
    name: 'Meetings',
    type: 'typed',
    description: '결정 사항과 액션 아이템을 회의록에서 자동 추출',
    fields: [
      { key: 'when', label: '일시' },
      { key: 'who', label: '상대' },
      { key: 'decision', label: '결정 사항' },
      { key: 'action', label: '액션 아이템' },
    ],
    records: [
      {
        id: 'meeting-630',
        title: '내부 회의 6/30',
        status: 'approved',
        values: {
          when: '6/30',
          who: '대표 · 재무팀 · 개발팀',
          decision: 'A클라우드 계약 조건 재확인',
          action: '법무팀 검토 후 수정 요청',
        },
        provenance: {
          decision: {
            sourceTitle: '회의록 6/30',
            quote: 'A클라우드 견적 검토 — 금액 이슈 재확인 필요.',
          },
          action: {
            sourceTitle: '회의록 6/30',
            quote: '계약서 9조 수정안 법무팀 송부 합의.',
          },
        },
      },
    ],
  },
];

export const INITIAL_LINKS: CcLink[] = [
  {
    id: 'link-contract-risk',
    kind: 'supportsField',
    from: 'contract-draft',
    to: 'contract-b',
    label: 'supports risk',
  },
  {
    id: 'link-meeting-crm',
    kind: 'belongsTo',
    from: 'meeting-note',
    to: 'crm-b',
    label: 'next action',
  },
];

export const SEEDED_CONFLICT: CcConflict = {
  id: 'conflict-price',
  fieldLabel: '월 인프라 비용',
  latestSource: 'A클라우드 최종 견적 메일',
  olderSource: '미팅 노트 6/24',
  latestQuote: '최종 견적은 월 340만 원',
  olderQuote: '월 320만 원으로 안내됨',
};

export const LEGAL_RISK_QUOTE =
  '9조 손해배상 상한 200%는 표준 계약 대비 높은 편이므로 100% 또는 150%로 하향 요청 필요.';
