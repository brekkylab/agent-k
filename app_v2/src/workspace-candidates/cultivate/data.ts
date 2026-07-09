// Workspace candidate D — the cultivation archetype ("Workspace 키우기/가꾸기").
// Concept source: docs/workspace-cultivation-direction.md
//   - material → evidence via declared, typed collections (schema declared, filling automated)
//   - collection #0 = the existing tantivy/BM25 knowledge index
//   - governance: structure changes are proposed → approved; filling & gap-flagging are automatic
//   - deposit loop: session artifacts flow back into the workspace
// Self-contained mock data; Korean copy hardcoded per mockup ground rules.

// ---------- knowledge units ----------

export interface CvField {
  key: string;
  label: string;
}

export interface CvRecord {
  id: string;
  values: Record<string, string>;
  /** provenance: field key → { sourceTitle, quote } */
  prov: Record<string, { sourceTitle: string; quote: string }>;
  /** set when the automatic gap-scanner flagged this record */
  gap?: string;
}

export interface CvCollection {
  id: string;
  icon: string;
  name: string;
  kind: 'index' | 'typed';
  fields: CvField[];       // typed only
  records: CvRecord[];     // typed only
  docCount?: number;       // index only
  syncLabel?: string;      // index only, Dust-style "last sync" row label
}

// ---------- collection #0 — the BM25 knowledge index ----------

export interface CvIndexDoc {
  id: string;
  icon: string;
  title: string;
  origin: string;
  indexedAt: string;
}

export const INDEX_DOCS: CvIndexDoc[] = [
  { id: 'ix-1', icon: '📄', title: '2026년 2분기 실적 보고서.pdf', origin: 'Google Drive', indexedAt: '2분 전' },
  { id: 'ix-2', icon: '✉️', title: 'Re: 클라우드 인프라 견적서 검토 요청', origin: 'Gmail', indexedAt: '2분 전' },
  { id: 'ix-3', icon: '📄', title: '신규 서비스 계약서 2026-07.docx', origin: 'Google Drive', indexedAt: '1시간 전' },
  { id: 'ix-4', icon: '📝', title: 'REST API 명세서 v3.1', origin: 'Confluence', indexedAt: '어제' },
  { id: 'ix-5', icon: '💬', title: 'v2.3.0 배포 완료 알림 스레드', origin: 'Slack', indexedAt: '어제' },
  { id: 'ix-6', icon: '📄', title: 'NDA 체결서 (A사).pdf', origin: 'Google Drive', indexedAt: '3일 전' },
  { id: 'ix-7', icon: '📋', title: '결제 모듈 성능 개선 티켓', origin: 'Jira', indexedAt: '3일 전' },
  { id: 'ix-8', icon: '📄', title: '마케팅 캠페인 기획서 Q3.pptx', origin: 'Google Drive', indexedAt: '5일 전' },
];

// ---------- typed collections (#1..N) ----------

export const MEETINGS: CvCollection = {
  id: 'col-meetings',
  icon: '🤝',
  name: '미팅',
  kind: 'typed',
  fields: [
    { key: 'when', label: '일시' },
    { key: 'who', label: '상대' },
    { key: 'decision', label: '결정 사항' },
    { key: 'action', label: '액션 아이템' },
  ],
  records: [
    {
      id: 'mt-1',
      values: {
        when: '6/30',
        who: 'B사 구매팀',
        decision: '계약 초안 7월 중 검토 완료 합의',
        action: '9조 손해배상 상한 수정안 송부',
      },
      prov: {
        decision: { sourceTitle: '회의록 6/30', quote: '양사는 7월 중 계약 초안 검토를 완료하기로 합의했다.' },
        action: { sourceTitle: '회의록 6/30', quote: '후속: 9조 상한 조정안을 우리 측이 초안해 송부한다.' },
      },
    },
    {
      id: 'mt-2',
      values: {
        when: '6/24',
        who: 'A클라우드 영업',
        decision: '3년 약정 15% 할인 제안 수령',
        action: '트래픽 전망치 산출 후 회신',
      },
      prov: {
        decision: { sourceTitle: '미팅 노트 6/24', quote: '3년 약정 시 15% 할인을 적용하겠다는 제안을 받았다.' },
        action: { sourceTitle: '미팅 노트 6/24', quote: '월 트래픽 전망을 산출해 다음 주까지 회신하기로 했다.' },
      },
    },
    {
      id: 'mt-3',
      values: {
        when: '6/18',
        who: '투자사 K파트너스',
        decision: '후속 미팅은 2분기 실적 확정 후',
        action: 'IR덱 업데이트',
      },
      prov: {
        decision: { sourceTitle: '미팅 노트 6/18', quote: '2분기 숫자가 확정되면 후속 미팅을 잡기로 했다.' },
        action: { sourceTitle: '미팅 노트 6/18', quote: 'IR 덱의 트랙션 페이지를 최신 수치로 갱신할 것.' },
      },
    },
  ],
};

export const CRM: CvCollection = {
  id: 'col-crm',
  icon: '📇',
  name: 'CRM',
  kind: 'typed',
  fields: [
    { key: 'name', label: '고객/딜' },
    { key: 'stage', label: '단계' },
    { key: 'lastTouch', label: '마지막 접점' },
    { key: 'next', label: '다음 액션' },
  ],
  records: [
    {
      id: 'crm-1',
      values: { name: 'B사 — 서비스 공급', stage: '계약 협상', lastTouch: '6/30 미팅', next: '9조 수정안 송부' },
      prov: {
        stage: { sourceTitle: '회의록 6/30', quote: '계약 초안 검토 단계로 진입.' },
        next: { sourceTitle: '회의록 6/30', quote: '후속: 9조 상한 조정안 송부.' },
      },
    },
    {
      id: 'crm-2',
      values: { name: 'C사 — 파일럿', stage: '제안', lastTouch: '6/12 메일', next: '데모 일정 확정' },
      prov: {
        lastTouch: { sourceTitle: '메일 6/12', quote: '파일럿 범위 제안서를 잘 받았다는 회신.' },
      },
      gap: '3주째 접점 없음 — 후속 필요',
    },
    {
      id: 'crm-3',
      values: { name: 'K파트너스 — 투자', stage: '팔로업 대기', lastTouch: '6/18 미팅', next: '실적 확정 후 재접촉' },
      prov: {
        next: { sourceTitle: '미팅 노트 6/18', quote: '2분기 숫자 확정 후 후속 미팅.' },
      },
    },
  ],
};

/** Contracts collection — NOT enabled at start; created when the user approves
 *  the structure proposal ("계약 템플릿을 켤까요?"). */
export const CONTRACTS_TEMPLATE: CvCollection = {
  id: 'col-contracts',
  icon: '📑',
  name: '계약',
  kind: 'typed',
  fields: [
    { key: 'party', label: '상대방' },
    { key: 'term', label: '기간' },
    { key: 'risk', label: '주의 조항' },
    { key: 'status', label: '상태' },
  ],
  records: [
    {
      id: 'ct-1',
      values: { party: 'B사', term: '12개월', risk: '9조 배상 상한 200%', status: '협상 중' },
      prov: {
        risk: {
          sourceTitle: '신규 서비스 계약서 2026-07.docx',
          quote: '제9조: 손해배상 책임 총액은 최근 3개월 대금의 100분의 200을 상한으로 한다.',
        },
      },
    },
    {
      id: 'ct-2',
      values: { party: 'A사 (NDA)', term: '24개월', risk: '—', status: '체결 완료' },
      prov: {
        term: { sourceTitle: 'NDA 체결서 (A사).pdf', quote: '본 계약의 유효기간은 체결일로부터 24개월로 한다.' },
      },
    },
  ],
};

// ---------- sources: pipes with a browsable catalog (pull side) ----------
// A source has two faces: the PIPE (auto-inflow status) and the CATALOG
// (what exists at the source — browsable, so the user can pull specific
// files in manually or mention them in a conversation without ingesting).

export interface CvCatalogItem {
  id: string;
  icon: string;
  title: string;
  modified: string;
  ingested: boolean; // already in the workspace index?
}

export interface CvSource {
  id: string;
  icon: string;
  name: string;
  status: 'live' | 'paused';
  lastSync: string;
  catalog: CvCatalogItem[];
}

export const SOURCES: CvSource[] = [
  {
    id: 'src-drive',
    icon: '🟦',
    name: 'Google Drive',
    status: 'live',
    lastSync: '2분 전',
    catalog: [
      { id: 'cat-d1', icon: '📄', title: '2026년 2분기 실적 보고서.pdf', modified: '7/01', ingested: true },
      { id: 'cat-d2', icon: '📄', title: '신규 서비스 계약서 2026-07.docx', modified: '6/29', ingested: true },
      { id: 'cat-d3', icon: '📄', title: 'NDA 체결서 (A사).pdf', modified: '6/27', ingested: true },
      { id: 'cat-d4', icon: '📊', title: '인프라 마이그레이션 계획서.xlsx', modified: '6/23', ingested: false },
      { id: 'cat-d5', icon: '📽️', title: '마케팅 캠페인 기획서 Q3.pptx', modified: '6/30', ingested: true },
      { id: 'cat-d6', icon: '📄', title: '보안 감사 보고서 2026-H1.pdf', modified: '6/26', ingested: false },
    ],
  },
  {
    id: 'src-gmail',
    icon: '✉️',
    name: 'Gmail',
    status: 'live',
    lastSync: '30분 전',
    catalog: [
      { id: 'cat-g1', icon: '✉️', title: 'Re: 클라우드 인프라 견적서 검토 요청', modified: '7/01', ingested: true },
      { id: 'cat-g2', icon: '✉️', title: 'C사 — 데모 일정 문의 회신', modified: '7/07', ingested: false },
      { id: 'cat-g3', icon: '✉️', title: '[법무] NDA 계약서 최종 검토 완료', modified: '6/26', ingested: false },
      { id: 'cat-g4', icon: '✉️', title: 'AWS Summit Seoul 2026 초청장', modified: '6/23', ingested: false },
    ],
  },
  {
    id: 'src-slack',
    icon: '💬',
    name: 'Slack',
    status: 'live',
    lastSync: '방금',
    catalog: [
      { id: 'cat-s1', icon: '💬', title: 'v2.3.0 배포 완료 알림 스레드', modified: '7/02', ingested: true },
      { id: 'cat-s2', icon: '💬', title: 'Rate Limiting 정책 구현 논의', modified: '6/26', ingested: false },
      { id: 'cat-s3', icon: '💬', title: 'Q3 로드맵 킥오프 준비 논의', modified: '6/29', ingested: false },
    ],
  },
  {
    id: 'src-jira',
    icon: '📋',
    name: 'Jira',
    status: 'paused',
    lastSync: '3일 전 (일시정지)',
    catalog: [
      { id: 'cat-j1', icon: '📋', title: '결제 모듈 성능 개선 티켓', modified: '7/02', ingested: true },
      { id: 'cat-j2', icon: '📋', title: 'v2.3.0 릴리스 태스크 트래킹', modified: '6/23', ingested: false },
    ],
  },
];

// ---------- inbox (incoming material awaiting triage) ----------

export interface CvInboxItem {
  id: string;
  icon: string;
  title: string;
  origin: string; // where it came from (source or session deposit)
  arrivedAt: string;
  suggestion: {
    target: string; // collection name, or '지식 인덱스'
    reason: string; // why the workspace proposes this
    /** record created on approval (typed collections only) */
    record?: {
      collectionId: string;
      values: Record<string, string>;
      provField: string;
      quote: string;
    };
  };
}

export const INITIAL_INBOX: CvInboxItem[] = [
  {
    id: 'in-1',
    icon: '📝',
    title: '회의록 7/04 — B사 킥오프 준비',
    origin: 'Google Drive · 자동 유입',
    arrivedAt: '오늘 09:12',
    suggestion: {
      target: '미팅',
      reason: '참석자·결정·후속 구조 감지 — 미팅 스키마와 일치',
      record: {
        collectionId: 'col-meetings',
        values: { when: '7/04', who: 'B사 실무진', decision: '킥오프 7/15 확정', action: '기술 요구사항 문서 공유' },
        provField: 'decision',
        quote: '킥오프 미팅은 7월 15일 오후 2시로 확정한다.',
      },
    },
  },
  {
    id: 'in-2',
    icon: '🧮',
    title: 'calc.py — 세션 산출물',
    origin: '세션 6c65148c · deposit',
    arrivedAt: '오늘 08:47',
    suggestion: {
      target: '지식 인덱스',
      reason: '세션이 만든 산출물 — 워크스페이스에 되심어 재사용 가능하게',
    },
  },
  {
    id: 'in-3',
    icon: '✉️',
    title: 'C사 — 데모 일정 문의 회신',
    origin: 'Gmail · 자동 유입',
    arrivedAt: '오늘 08:30',
    suggestion: {
      target: 'CRM',
      reason: '기존 딜 "C사 — 파일럿"의 접점 공백(3주)을 해소하는 메일',
      record: {
        collectionId: 'col-crm',
        values: { name: 'C사 — 파일럿', stage: '제안', lastTouch: '7/07 메일', next: '데모 7/10 제안' },
        provField: 'next',
        quote: '데모는 7월 10일 오전이 가능하다는 회신을 받았다.',
      },
    },
  },
  {
    id: 'in-4',
    icon: '📄',
    title: '제품 로드맵 v2.pptx',
    origin: '업로드',
    arrivedAt: '어제',
    suggestion: {
      target: '지식 인덱스',
      reason: '참고 자료 성격 — 타입 컬렉션 대상 아님, 인덱스로 색인',
    },
  },
];

// ---------- structure proposal (governance: propose → approve) ----------

export const STRUCTURE_PROPOSAL = {
  id: 'prop-contracts',
  title: '계약 문서 3건이 쌓였어요 — 「계약」 컬렉션을 켤까요?',
  reason:
    '신규 서비스 계약서·NDA·견적 메일에서 계약 구조(상대방·기간·조항)가 반복 감지됐습니다. ' +
    '켜면 선언된 스키마(상대방/기간/주의 조항/상태)에 맞춰 자동으로 채워집니다.',
  detected: ['신규 서비스 계약서 2026-07.docx', 'NDA 체결서 (A사).pdf', 'Re: 클라우드 인프라 견적서'],
};

// ---------- growth digest ----------

export interface CvDigest {
  inflow: number;   // 키우기: new material
  promoted: number; // 가꾸기: material → evidence
  conflicts: number;
  gaps: number;
}

export const INITIAL_DIGEST: CvDigest = { inflow: 4, promoted: 0, conflicts: 1, gaps: 1 };

/** The single seeded conflict — surfaces the "충돌 배지" whitespace. */
export const CONFLICT = {
  title: '견적 금액 불일치',
  a: { sourceTitle: 'Re: 클라우드 인프라 견적서 (7/01)', quote: '월 요금 340만 원 기준이며…' },
  b: { sourceTitle: '미팅 노트 6/24', quote: '월 320만 원 수준으로 안내받음' },
  hint: '최신 메일(7/01) 기준이 유력 — 미팅 노트가 구버전 견적을 참조 중',
};

// ---------- ask-in-context (conversation as a verb) canned replies ----------

export function contextAnswer(question: string, context: string): string {
  if (/공백|접점|후속/.test(question) || /C사/.test(context)) {
    return (
      'C사 딜은 6/12 이후 접점이 없었는데, 오늘 인박스에 데모 일정 회신이 들어와 있어요. ' +
      '승인하면 마지막 접점이 7/07로 갱신됩니다. 후속 메일 초안이 필요하면 바로 만들어 드릴게요.'
    );
  }
  if (/9조|계약|위험|리스크/.test(question) || /계약/.test(context)) {
    return (
      'B사 계약의 핵심 리스크는 9조 — 배상 상한이 "3개월 대금의 200%"로 표준보다 우리에게 불리합니다. ' +
      '6/30 미팅에서 수정안 송부를 약속했으니, 계약 레코드의 다음 액션과 연결돼 있어요.'
    );
  }
  return (
    `"${context}" 맥락에서 답변드려요 — 이 컬렉션의 레코드와 출처 구절을 근거로 사용했습니다. ` +
    '더 구체적인 필드를 지정하면 해당 출처 대목을 인용해 드릴게요.'
  );
}
