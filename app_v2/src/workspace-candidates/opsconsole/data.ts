// Workspace candidate F — the ops console archetype ("Glean/Dust 운영 콘솔").
// Source-centric extreme: connections, sync freshness, indexing status, and a
// verification loop that keeps the corpus trustworthy.
// Philosophy layer (docs/workspace-cultivation-direction.md):
//   material → evidence (근거화), deposit loop (session artifacts as 1st-class source),
//   conflict detection — all expressed as ops-console native concepts.
// Self-contained mock data; Korean copy hardcoded per mockup ground rules.

// ---------- indexing / verify states ----------

export type IndexState = 'pending' | 'indexing' | 'done' | 'failed';
export type VerifyState = 'verified' | 'unverified' | 'deprecated';

// ---------- evidence item (근거화 — material → evidence) ----------

export interface FEvidence {
  /** target collection name, e.g. "계약" */
  collection: string;
  /** field within that collection, e.g. "주의 조항" */
  field: string;
  /** extracted value */
  value: string;
}

// ---------- usage timeline entry (세션 참조 기록) ----------

export interface FUsage {
  when: string;   // e.g. "7/02"
  what: string;   // e.g. "세션 6c65 인용"
}

// ---------- document ----------

export interface FDoc {
  id: string;
  icon: string;
  title: string;
  indexState: IndexState;
  /** 0–1, present when indexState === 'indexing' */
  indexPct?: number;
  verify: VerifyState;
  /** ISO date string */
  verifiedAt?: string;
  /** ISO date string — when the verification expires */
  expiresAt?: string;
  /** human-readable last-used label */
  lastUsed?: string;
  /** higher = more urgent to reverify */
  urgency?: number;
  /** body preview — 3-4 paragraphs of document excerpt */
  body?: string;
  /** extracted evidence items (근거화됨) */
  evidence?: FEvidence[];
  /** related doc ids for cross-navigation */
  related?: string[];
  /** session usage timeline */
  usage?: FUsage[];
}

// ---------- source ----------

export interface FSource {
  id: string;
  icon: string;
  name: string;
  status: 'live' | 'paused';
  lastSync: string;
  docs: FDoc[];
}

// ---------- conflict card (inbox item type) ----------

export interface FConflict {
  id: string;
  kind: 'conflict';
  title: string;
  hint: string;
  docA: { id: string; sourceTitle: string; quote: string };
  docB: { id: string; sourceTitle: string; quote: string };
  urgency: number;
}

// ---------- queue item union (doc or conflict) ----------

export interface FDocQueueItem extends FDoc {
  kind: 'doc';
  sourceName: string;
}

export type FQueueItem = FDocQueueItem | FConflict;

// ---------- seed data ----------

export const INITIAL_SOURCES: FSource[] = [
  {
    id: 'src-drive',
    icon: '🟦',
    name: 'Google Drive',
    status: 'live',
    lastSync: '2분 전',
    docs: [
      {
        id: 'doc-d1',
        icon: '📄',
        title: '2026년 2분기 실적 보고서.pdf',
        indexState: 'done',
        verify: 'verified',
        verifiedAt: '2026-06-28',
        expiresAt: '2026-09-28',
        lastUsed: '1시간 전',
        urgency: 3,
        body:
          '2026년 2분기(4월–6월) 사업 실적을 정리한 보고서입니다. ' +
          '매출은 전년 동기 대비 23% 성장한 12억 4천만 원을 기록했으며, 영업이익률은 18.4%로 개선되었습니다.\n\n' +
          '주요 성장 동인은 엔터프라이즈 고객 신규 계약 3건(B사·C사·K파트너스)과 SaaS 구독 갱신율 94%입니다. ' +
          '비용 측면에서는 인프라 마이그레이션 완료로 운영비가 8% 절감되었습니다.\n\n' +
          'Q3 가이던스: 매출 13–14억, 신규 파이프라인 5건 이상 목표. IR 덱은 실적 확정 후 K파트너스에 공유 예정.',
        evidence: [
          { collection: 'CRM', field: '트랙션', value: '매출 23% YoY 성장, 영업이익률 18.4%' },
        ],
        related: ['doc-d2', 'doc-g1'],
        usage: [
          { when: '7/04', what: '세션 a3f2 IR 덱 초안 작성에 인용' },
          { when: '7/01', what: '세션 6c65 Q3 가이던스 산출에 참조' },
          { when: '6/30', what: '미팅 노트 6/30 배경자료로 사용' },
        ],
      },
      {
        id: 'doc-d2',
        icon: '📄',
        title: '신규 서비스 계약서 2026-07.docx',
        indexState: 'done',
        verify: 'unverified',
        lastUsed: '2일 전',
        urgency: 87,
        body:
          'B사와 체결 예정인 서비스 공급 계약서 초안입니다. 계약 기간 12개월, 자동 갱신 조항 포함.\n\n' +
          '핵심 리스크 조항 — 제9조(손해배상): "본 계약과 관련하여 발생하는 손해배상 총액은 ' +
          '계약 체결 전 3개월 간 수령한 대금의 100분의 200을 상한으로 한다." 표준 계약 대비 불리한 조건.\n\n' +
          '제12조(비밀유지): 계약 종료 후 5년간 유효. 제15조(준거법): 대한민국 법률 적용, ' +
          '분쟁 시 서울중앙지방법원 전속 관할.',
        evidence: [
          { collection: '계약', field: '주의 조항', value: '9조 배상 상한 200% — 표준 대비 불리' },
          { collection: '계약', field: '상태', value: '협상 중' },
        ],
        related: ['doc-d1', 'doc-d3'],
        usage: [
          { when: '7/06', what: '세션 b8e1 계약 리스크 분석에 참조' },
          { when: '7/02', what: '세션 6c65 9조 수정안 초안 작성 시 원본 참조' },
        ],
      },
      {
        id: 'doc-d3',
        icon: '📄',
        title: 'NDA 체결서 (A사).pdf',
        indexState: 'done',
        verify: 'verified',
        verifiedAt: '2026-05-10',
        expiresAt: '2026-08-10',
        lastUsed: '5일 전',
        urgency: 12,
        body:
          'A사와 체결한 비밀유지계약서(Non-Disclosure Agreement). 유효기간 24개월(2024-05-10–2026-05-10).\n\n' +
          '보호 대상 정보: 기술 정보, 사업 계획, 고객 정보, 재무 데이터. 정보 수령 후 30일 이내 서면 표시 의무.\n\n' +
          '주의: 유효기간이 2026-08-10에 만료됩니다. 갱신 또는 신규 체결 여부를 법무팀과 협의 필요.',
        related: ['doc-d2'],
        usage: [
          { when: '6/28', what: '세션 4d77 A사 후속 협력 검토 시 참조' },
        ],
      },
      {
        id: 'doc-d4',
        icon: '📊',
        title: '마케팅 캠페인 기획서 Q3.pptx',
        indexState: 'indexing',
        indexPct: 0.6,
        verify: 'unverified',
        urgency: 20,
        body:
          'Q3(7–9월) 마케팅 캠페인 전략 기획서. 핵심 채널: LinkedIn B2B 광고, 세미나 2회, 파트너 레퍼럴.\n\n' +
          '예산: 3,800만 원(디지털 광고 60%, 오프라인 이벤트 30%, 콘텐츠 10%). KPI: MQL 120건 이상.\n\n' +
          '(인덱싱 진행 중 — 전체 내용은 완료 후 확인 가능)',
      },
      {
        id: 'doc-d5',
        icon: '📄',
        title: '보안 감사 보고서 2026-H1.pdf',
        indexState: 'failed',
        verify: 'unverified',
        urgency: 55,
        body: '(인덱싱 실패 — 재시도 후 내용 확인 가능)',
      },
    ],
  },
  {
    id: 'src-gmail',
    icon: '✉️',
    name: 'Gmail',
    status: 'live',
    lastSync: '30분 전',
    docs: [
      {
        id: 'doc-g1',
        icon: '✉️',
        title: 'Re: 클라우드 인프라 견적서 검토 요청',
        indexState: 'done',
        verify: 'verified',
        verifiedAt: '2026-07-01',
        expiresAt: '2026-10-01',
        lastUsed: '방금',
        urgency: 5,
        body:
          '발신: A클라우드 영업팀 (sales@acloud.kr) · 2026-07-01 14:22\n\n' +
          '안녕하세요. 말씀드린 3년 약정 클라우드 서비스 견적서를 첨부 드립니다. ' +
          '월 이용료는 340만 원 기준이며, 3년 약정 시 15% 할인 적용 가능합니다.\n\n' +
          '트래픽 전망치와 스토리지 요구사항을 공유해 주시면 최종 견적을 조정해 드리겠습니다.',
        evidence: [
          { collection: 'CRM', field: '견적', value: '월 340만 원 (3년 약정 15% 할인 가능)' },
        ],
        related: ['doc-d1'],
        usage: [
          { when: '7/03', what: '세션 9a12 인프라 비용 비교 분석에 인용' },
          { when: '7/01', what: '세션 6c65 트래픽 전망 산출 시 참조' },
        ],
      },
      {
        id: 'doc-g2',
        icon: '✉️',
        title: 'C사 — 데모 일정 문의 회신',
        indexState: 'done',
        verify: 'unverified',
        lastUsed: '어제',
        urgency: 62,
        body:
          '발신: 이민준 (mjlee@ccompany.kr) · 2026-07-07 10:14\n\n' +
          '안녕하세요. 저번에 보내주신 파일럿 제안서 잘 받았습니다. ' +
          '데모는 7월 10일 오전이 가능합니다. 오전 10시에 화상 미팅으로 진행하면 어떨까요?\n\n' +
          '참석자는 저와 기술 검토 담당자 1명입니다. 접속 링크 공유 부탁드립니다.',
        evidence: [],
        related: [],
        usage: [{ when: '7/07', what: '세션 c5f3 C사 후속 대응 초안에 인용' }],
      },
      {
        id: 'doc-g3',
        icon: '✉️',
        title: '[법무] NDA 계약서 최종 검토 완료',
        indexState: 'done',
        verify: 'deprecated',
        verifiedAt: '2026-03-15',
        lastUsed: '3주 전',
        urgency: 2,
        body:
          '발신: 법무팀 (legal@internal) · 2026-03-15\n\n' +
          'A사 NDA 최종 검토 완료. 주요 수정사항 없음. 서명 진행 가능합니다.\n\n' +
          '(구식: 이후 NDA 체결서 원본으로 대체됨)',
        related: ['doc-d3'],
        usage: [],
      },
    ],
  },
  {
    id: 'src-slack',
    icon: '💬',
    name: 'Slack',
    status: 'live',
    lastSync: '방금',
    docs: [
      {
        id: 'doc-s1',
        icon: '💬',
        title: 'v2.3.0 배포 완료 알림 스레드',
        indexState: 'done',
        verify: 'verified',
        verifiedAt: '2026-07-02',
        expiresAt: '2026-10-02',
        lastUsed: '3시간 전',
        urgency: 4,
        body:
          '#deploy-announce · 2026-07-02 16:45\n\n' +
          'v2.3.0 배포 완료. 주요 변경: Rate Limiting v2(토큰 버킷), 결제 모듈 응답속도 38% 개선, ' +
          'UI 다크모드 베타.\n\n' +
          '롤백 포인트: v2.2.4 (Docker 이미지 태그 유지). 이상 감지 시 #incident 채널 알림.',
        evidence: [
          { collection: '릴리스', field: '버전', value: 'v2.3.0 — 2026-07-02 배포 완료' },
        ],
        related: ['doc-s2'],
        usage: [
          { when: '7/04', what: '세션 a3f2 릴리스 노트 요약 생성에 참조' },
        ],
      },
      {
        id: 'doc-s2',
        icon: '💬',
        title: 'Rate Limiting 정책 구현 논의',
        indexState: 'done',
        verify: 'unverified',
        lastUsed: '3일 전',
        urgency: 38,
        body:
          '#backend-dev · 2026-06-26\n\n' +
          '토큰 버킷 vs 슬라이딩 윈도우 방식 논의. 최종: 토큰 버킷 채택 (구현 단순성 + 버스트 허용).\n\n' +
          '기본 한도: 1,000 req/min per API key, 버스트 최대 1.5배. 초과 시 429 응답 + Retry-After 헤더.',
        related: ['doc-s1'],
        usage: [{ when: '6/28', what: '세션 7b3e API 설계 검토 시 인용' }],
      },
      {
        id: 'doc-s3',
        icon: '💬',
        title: 'Q3 로드맵 킥오프 준비 논의',
        indexState: 'pending',
        verify: 'unverified',
        urgency: 15,
        body: '(인덱싱 대기 중 — 내용 확인 불가)',
      },
    ],
  },
  {
    id: 'src-jira',
    icon: '📋',
    name: 'Jira',
    status: 'paused',
    lastSync: '3일 전',
    docs: [
      {
        id: 'doc-j1',
        icon: '📋',
        title: '결제 모듈 성능 개선 티켓',
        indexState: 'done',
        verify: 'unverified',
        lastUsed: '6시간 전',
        urgency: 73,
        body:
          'JIRA-482 · 담당: 김개발 · 우선순위: P1\n\n' +
          '문제: 결제 API 평균 응답시간 820ms (목표 500ms 이하). 병목: DB 쿼리 N+1 및 캐시 미스율 높음.\n\n' +
          '해결: 쿼리 최적화 + Redis 캐시 레이어 추가. v2.3.0에 완료 (응답시간 38% 개선, 510ms → 316ms).\n\n' +
          '현재 이 티켓의 수치가 최신 배포 결과(v2.3.0)와 일치하는지 검증 필요.',
        evidence: [
          { collection: '릴리스', field: '성능 지표', value: '결제 API 316ms (v2.3.0)' },
        ],
        related: ['doc-s1'],
        usage: [
          { when: '7/07', what: '세션 d9f1 성능 벤치마크 보고서 작성 시 참조' },
          { when: '7/02', what: '세션 6c65 v2.3.0 릴리스 노트 근거로 인용' },
        ],
      },
      {
        id: 'doc-j2',
        icon: '📋',
        title: 'v2.3.0 릴리스 태스크 트래킹',
        indexState: 'done',
        verify: 'deprecated',
        verifiedAt: '2026-06-20',
        lastUsed: '2주 전',
        urgency: 8,
        body:
          'JIRA-470 · v2.3.0 출시 체크리스트. 전체 태스크 27개 중 27개 완료.\n\n' +
          '(구식: v2.3.0 배포 완료로 이 티켓은 아카이브됨. 운영 이슈는 #incident 채널로)',
        related: ['doc-s1'],
        usage: [],
      },
    ],
  },
  // --- Transformation 4: session artifacts as 1st-class source (deposit loop) ---
  {
    id: 'src-sessions',
    icon: '🧮',
    name: '세션 산출물',
    status: 'live',
    lastSync: '세션 종료 시',
    docs: [
      {
        id: 'doc-sess1',
        icon: '🧮',
        title: 'calc.py — 트래픽 전망 산출 스크립트',
        indexState: 'done',
        verify: 'unverified',
        lastUsed: '어제',
        urgency: 68,
        body:
          '세션 6c65에서 생성된 산출물. 인프라 트래픽 전망치를 월별로 산출하는 Python 스크립트.\n\n' +
          '입력: 현재 DAU, 성장률 %, 피크 배수. 출력: 월별 req/min 전망 + 권장 인스턴스 스펙.\n\n' +
          '이 스크립트는 A클라우드 견적 협상의 트래픽 근거로 사용됨. 재사용 가능한 자산으로 워크스페이스에 deposit됨.',
        evidence: [],
        related: ['doc-g1'],
        usage: [
          { when: '7/01', what: '세션 6c65에서 생성 (deposit)' },
          { when: '7/03', what: '세션 9a12 인프라 비용 분석에 재사용' },
        ],
      },
      {
        id: 'doc-sess2',
        icon: '📝',
        title: '9조 수정안 초안.md',
        indexState: 'done',
        verify: 'unverified',
        lastUsed: '2일 전',
        urgency: 82,
        body:
          '세션 b8e1에서 생성된 계약 9조 수정안. B사 계약서의 배상 상한 조항을 표준 조건(100%)으로 낮추는 초안.\n\n' +
          '"제9조(손해배상): 본 계약에 따른 손해배상 총액은 해당 월 이용료의 100분의 100을 상한으로 한다. ' +
          '단, 고의 또는 중과실에 의한 손해의 경우는 제외한다."\n\n' +
          '법무 검토 전 단계. 이 문서를 계약 컬렉션의 "수정안" 필드로 근거화하면 협상 추적이 가능.',
        evidence: [],
        related: ['doc-d2'],
        usage: [
          { when: '7/02', what: '세션 b8e1에서 생성 (deposit)' },
          { when: '7/06', what: '세션 b8e1 B사 협상 자료 패키지에 포함' },
        ],
      },
    ],
  },
];

// ---------- seeded conflict card (Transformation 3) ----------

export const SEEDED_CONFLICT: FConflict = {
  id: 'conflict-1',
  kind: 'conflict',
  title: '클라우드 인프라 견적 금액 불일치',
  // Aligned with B/C candidates: the 7/01 re-adjusted mail (320) is the latest;
  // the earlier 340 quote (doc-g1) is the outdated version.
  hint: '최신 재조정 메일(7/01) 기준 320만 유력 — 최초 견적서(340만)가 구버전',
  docA: {
    id: 'doc-d1',
    sourceTitle: '재조정 견적 회신 (7/01 최신)',
    quote: '최종 조정 단가는 월 320만 원이며, 초과 과금은 월 50만 원을 상한으로 합니다.',
  },
  docB: {
    id: 'doc-g1',
    sourceTitle: 'Re: 클라우드 인프라 견적서 (최초 340만 원)',
    quote: '월 이용료는 340만 원 기준이며, 3년 약정 시 15% 할인 적용 가능합니다.',
  },
  urgency: 95,
};

// ---------- trust score ----------

export const TRUST_INITIAL = 71;

/** Recompute trust: verified & indexState=done docs / total docs, as percentage. */
export function computeTrust(sources: FSource[]): number {
  const allDocs = sources.flatMap((s) => s.docs);
  if (allDocs.length === 0) return 0;
  const healthy = allDocs.filter((d) => d.verify === 'verified' && d.indexState === 'done').length;
  return Math.round((healthy / allDocs.length) * 100);
}

/** Recompute evidence rate (근거화율): docs with evidence / total done docs. */
export function computeEvidenceRate(sources: FSource[]): number {
  const doneDocs = sources.flatMap((s) => s.docs).filter((d) => d.indexState === 'done');
  if (doneDocs.length === 0) return 0;
  const evidenced = doneDocs.filter((d) => d.evidence && d.evidence.length > 0).length;
  return Math.round((evidenced / doneDocs.length) * 100);
}

// ---------- verify queue (with conflict card) ----------

/** Top urgent queue items: doc (unverified/deprecated) + conflict cards. */
export function deriveVerifyQueue(
  sources: FSource[],
  showConflict: boolean,
): FQueueItem[] {
  const docItems: FDocQueueItem[] = sources
    .flatMap((s) =>
      s.docs
        .filter((d) => d.verify === 'unverified' || d.verify === 'deprecated')
        .map((d): FDocQueueItem => ({ ...d, kind: 'doc', sourceName: s.name })),
    )
    .sort((a, b) => (b.urgency ?? 0) - (a.urgency ?? 0))
    .slice(0, 5);

  const items: FQueueItem[] = showConflict
    ? [SEEDED_CONFLICT, ...docItems]
    : docItems;

  return items;
}

/** Human-readable urgency label shown in the inbox card. */
export function urgencyLabel(doc: FDoc, sourceName: string): string {
  const age = doc.urgency ?? 0;
  const uses = age > 60 ? '12회' : age > 30 ? '6회' : '3회';
  const days = age > 60 ? '89일' : age > 30 ? '45일' : '21일';
  return `${sourceName} · ${uses} 참조 · ${days} 경과`;
}

// ---------- sparkline seed (7 bars, today is index 6) ----------

export const SPARKLINE_SEED = [55, 58, 61, 64, 66, 69, TRUST_INITIAL];

// ---------- helper: find doc across all sources ----------

export function findDoc(sources: FSource[], docId: string): { doc: FDoc; source: FSource } | null {
  for (const s of sources) {
    const doc = s.docs.find((d) => d.id === docId);
    if (doc) return { doc, source: s };
  }
  return null;
}
