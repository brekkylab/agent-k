// Mock data for workspace candidate B — the NotebookLM archetype.
// Self-contained on purpose: candidates demo flows, not backend state.
// Korean copy is hardcoded per the mockup ground rules.

export type NbSourceType =
  | 'pdf' | 'sheet' | 'mail' | 'ticket' | 'doc' | 'chat' | 'link' | 'text' | 'slides' | 'session';

export const NB_TYPE_ICON: Record<NbSourceType, string> = {
  pdf: '📄', sheet: '📊', mail: '✉️', ticket: '📋', doc: '📝',
  chat: '💬', link: '🔗', text: '✏️', slides: '📽️', session: '🗂️',
};

// Source kind (integration channel) — used for filter chips.
export type NbSourceKind = 'drive' | 'gmail' | 'slack' | 'jira' | 'upload' | 'session';

// Group label for left-pane grouping.
export type NbSourceGroup = '재무' | '계약' | '제품' | '받은 재료';

export interface NbSourceFreshness {
  state: 'ok' | 'stale';
  label: string; // e.g. '3일 전 확인' / '89일 경과'
}

export interface NbSource {
  id: string;
  type: NbSourceType;
  sourceKind: NbSourceKind;
  group: NbSourceGroup;
  title: string;
  origin: string;       // provenance label, e.g. 'Google Drive'
  modifiedAt: string;   // display string
  summary: string;      // auto summary shown in the Source Guide
  topics: string[];     // key-topic chips (click → chat question)
  quote: string;        // representative passage highlighted on citation jump
  // Extended Guide fields
  body?: string;        // 3-4 paragraphs; quote appears verbatim inside
  facts?: { label: string; value: string }[]; // extracted fact rows for the guide
  fresh?: NbSourceFreshness;
  related?: string[];   // source ids shown in "관련 문서"
  citedBy?: string[];   // seed answer snippets; grows at runtime via session citations
  analyzing?: boolean;  // freshly-added: summary/topics not ready yet
  // Deposit-loop markers
  unconfirmed?: boolean; // '받은 재료' initially unchecked + tagged "미확인"
}

export const INITIAL_SOURCES: NbSource[] = [
  {
    id: 'nb-quarter-report',
    type: 'pdf',
    sourceKind: 'drive',
    group: '재무',
    title: '2026년 2분기 실적 보고서.pdf',
    origin: 'Google Drive',
    modifiedAt: '7월 1일',
    fresh: { state: 'ok', label: '3일 전 확인' },
    summary:
      '2분기 매출은 전분기 대비 18% 성장한 4.2억 원. 신규 고객 32곳 확보가 주요 동력이며, ' +
      '클라우드 인프라 비용 증가(+22%)가 영업이익률을 2.1%p 잠식했다. 3분기 목표는 매출 5억 원.',
    topics: ['매출 성장 요인', '인프라 비용 증가', '3분기 목표'],
    quote:
      '클라우드 인프라 비용은 사용량 급증으로 전분기 대비 22% 증가했으며, 3분기 내 최적화가 필요하다.',
    body:
      '2분기 전체 매출은 4.2억 원으로 전분기(3.56억 원) 대비 18% 성장했다. 신규 고객 32곳이 전체 성장의 73%를 견인했으며, 기존 고객 확장 매출(Expansion MRR)은 아직 미미하다.\n\n' +
      '영업비용 구조에서 클라우드 인프라 비용은 사용량 급증으로 전분기 대비 22% 증가했으며, 3분기 내 최적화가 필요하다. 이로 인해 영업이익률은 12.4%에서 10.3%로 2.1%p 하락했다.\n\n' +
      '헤드카운트는 분기 중 6명 증원(총 41명)했으며, R&D 비율은 매출의 28%를 유지하고 있다. 3분기 목표는 매출 5억 원이며 인프라 비용을 현 수준에서 동결하는 것이 최우선 과제다.',
    facts: [
      { label: '2분기 매출', value: '4.2억 원 (+18%)' },
      { label: '신규 고객', value: '32곳' },
      { label: '인프라 비용 증가', value: '+22%' },
      { label: '3분기 목표', value: '5억 원' },
    ],
    related: ['nb-infra-quote-mail', 'nb-deploy-thread'],
    citedBy: ['2분기 매출은 4.2억 원으로 전분기 대비 18% 성장했고'],
  },
  {
    id: 'nb-infra-quote-mail',
    type: 'mail',
    sourceKind: 'gmail',
    group: '재무',
    title: 'Re: 클라우드 인프라 견적서 검토 요청',
    origin: 'Gmail',
    modifiedAt: '7월 1일',
    fresh: { state: 'ok', label: '1일 전 확인' },
    summary:
      'A클라우드 영업팀이 보낸 연간 계약 견적. 월 340만 원(3년 약정 시 15% 할인). ' +
      '단, 트래픽 초과분 과금 조항과 중도 해지 위약금 조건이 본문에 포함돼 있어 검토 필요.',
    topics: ['연 계약 할인 조건', '초과 과금 조항', '해지 위약금'],
    quote:
      '월 요금 340만 원 기준이며, 월간 트래픽 10TB 초과분에 대해서는 GB당 120원이 별도 과금됩니다.',
    body:
      '7월 1일 A클라우드 영업팀으로부터 수신한 연간 계약 견적 메일이다. 제안 요금은 월 340만 원(세전)이며, 3년 약정 시 15% 할인이 적용되어 월 289만 원(약정 기간 선불 조건)이다.\n\n' +
      '월 요금 340만 원 기준이며, 월간 트래픽 10TB 초과분에 대해서는 GB당 120원이 별도 과금됩니다. 또한 중도 해지 시 잔여 약정 기간 요금의 20%가 위약금으로 부과된다.\n\n' +
      '현재 실적 보고서 기준 인프라 비용 증가 추세와 결합하면 초과 과금 조항이 실제로 발동될 가능성을 검토해야 한다. 법무 확인이 필요한 위약금 조항과 함께 다음 주 미팅 안건으로 올릴 것을 권한다.',
    facts: [
      { label: '기본 월 요금', value: '340만 원' },
      { label: '3년 약정 할인', value: '15% (월 289만 원)' },
      { label: '초과 과금', value: 'GB당 120원 (10TB 초과 시)' },
      { label: '중도 해지 위약금', value: '잔여 기간 요금의 20%' },
    ],
    related: ['nb-quarter-report', 'nb-infra-quote-mail-v2'],
    citedBy: [],
  },
  {
    id: 'nb-infra-quote-mail-v2',
    type: 'mail',
    sourceKind: 'gmail',
    group: '재무',
    title: 'Re(2): 인프라 견적 최종 확인 — 단가 재조정',
    origin: 'Gmail',
    modifiedAt: '7월 1일',
    fresh: { state: 'ok', label: '1일 전 확인' },
    summary:
      '후속 메일로 A클라우드가 단가를 월 320만 원으로 재조정. 할인 조건은 동일하며 초과 과금 상한을 월 50만 원으로 캡핑하는 조항이 추가됐다.',
    topics: ['단가 재조정', '초과 과금 상한'],
    quote: '최종 조정 단가는 월 320만 원이며, 초과 과금은 월 50만 원을 상한으로 합니다.',
    body:
      '7월 1일 오후 A클라우드 담당자가 추가 메일로 견적을 수정 제안했다. 최초 견적(340만 원)에서 단가를 월 320만 원으로 낮추었다.\n\n' +
      '최종 조정 단가는 월 320만 원이며, 초과 과금은 월 50만 원을 상한으로 합니다. 나머지 조건(위약금 20%, 3년 약정 15% 할인)은 동일하다.\n\n' +
      '최초 견적 메일(340만 원)과 이 메일(320만 원)이 동일 날짜에 존재하므로, 견적 협상 과정에서 어느 버전이 유효한지 반드시 확인이 필요하다.',
    facts: [
      { label: '재조정 단가', value: '320만 원 / 월' },
      { label: '초과 과금 상한', value: '월 50만 원' },
    ],
    related: ['nb-infra-quote-mail'],
    citedBy: [],
  },
  {
    id: 'nb-service-contract',
    type: 'pdf',
    sourceKind: 'drive',
    group: '계약',
    title: '신규 서비스 계약서 2026-07.docx',
    origin: 'Google Drive',
    modifiedAt: '6월 29일',
    fresh: { state: 'ok', label: '5일 전 확인' },
    summary:
      'B사와의 서비스 공급 계약 초안. 계약 기간 12개월, 대금 분기별 후불. ' +
      '9조(손해배상 상한)와 12조(데이터 반환 의무)가 지난 표준계약 대비 우리 쪽에 불리하게 수정됨.',
    topics: ['손해배상 상한', '데이터 반환 의무', '대금 지급 조건'],
    quote:
      '제9조: 공급자의 손해배상 책임 총액은 최근 3개월 대금의 100분의 200을 상한으로 한다.',
    body:
      'B사와의 서비스 공급 계약서 초안(2026-07 버전)이다. 계약 기간은 12개월, 대금은 분기별 후불 조건이다.\n\n' +
      '제9조: 공급자의 손해배상 책임 총액은 최근 3개월 대금의 100분의 200을 상한으로 한다. 이는 기존 표준계약(100분의 100)의 두 배로, 우리 쪽 부담이 증가한다.\n\n' +
      '12조는 계약 종료 후 30일 이내 모든 데이터를 암호화된 형태로 반환 또는 삭제해야 하며, 삭제 확인서를 공증 받아 제출하는 조항이 신설됐다. 이 비용은 공급자(우리) 부담이다.\n\n' +
      '법무팀에 9조·12조 두 조항의 재협상 가능성 검토를 의뢰했으며, 협상 데드라인은 7월 10일이다.',
    facts: [
      { label: '계약 기간', value: '12개월' },
      { label: '대금 지급', value: '분기별 후불' },
      { label: '배상 상한', value: '3개월 대금의 200%' },
      { label: '협상 데드라인', value: '7월 10일' },
    ],
    related: ['nb-payment-ticket'],
    citedBy: [],
  },
  {
    id: 'nb-partner-nda',
    type: 'pdf',
    sourceKind: 'drive',
    group: '계약',
    title: 'C파트너 NDA 초안 v2.pdf',
    origin: 'Google Drive',
    modifiedAt: '6월 12일',
    fresh: { state: 'stale', label: '26일 경과' },
    summary:
      'C파트너와의 비밀유지 계약서 2차 초안. 유효기간 3년, 잔존 조항 5년. 상호 NDA 구조. 아직 최종 서명 전이며 6월 12일 이후 내부 검토가 멈춰 있음.',
    topics: ['잔존 기간', '상호 NDA 조건', '서명 일정'],
    quote: '본 계약상 비밀유지 의무는 계약 종료 후 5년간 존속한다.',
    body:
      'C파트너와의 상호 NDA 초안 2차 버전이다. 유효기간은 3년이며 계약 종료 이후에도 5년간 잔존 의무가 유지된다.\n\n' +
      '본 계약상 비밀유지 의무는 계약 종료 후 5년간 존속한다. 비밀정보의 정의는 서면 표시 여부와 무관하게 구두 공개도 포함하도록 넓게 규정됐다.\n\n' +
      '6월 12일 이후 내부 법무 검토가 정지 상태이다. 24일이 경과한 현재까지 C파트너로부터 후속 연락이 없으며, 계약 진행 상태 확인이 필요하다.',
    facts: [
      { label: '유효기간', value: '3년' },
      { label: '잔존 의무', value: '종료 후 5년' },
      { label: '마지막 갱신', value: '6월 12일' },
    ],
    related: ['nb-service-contract'],
    citedBy: [],
  },
  {
    id: 'nb-payment-ticket',
    type: 'ticket',
    sourceKind: 'jira',
    group: '제품',
    title: '결제 모듈 성능 개선 (응답 시간 50% 단축)',
    origin: 'Jira',
    modifiedAt: '7월 2일',
    fresh: { state: 'ok', label: '오늘 확인' },
    summary:
      '결제 API p95 응답이 1.8s로 악화되어 개선 착수. 원인은 외부 PG사 호출 직렬화. ' +
      '병렬화 + 캐시 도입으로 0.9s 목표. 현재 코드 리뷰 단계이며 배포는 v2.3.1 예정.',
    topics: ['성능 저하 원인', '개선 방식', '배포 일정'],
    quote: 'PG사 인증·승인 호출이 직렬로 수행되어 전체 지연의 61%를 차지한다.',
    body:
      '결제 API p95 응답 시간이 1.8s까지 상승해 성능 개선 티켓(PAY-412)이 생성됐다.\n\n' +
      'PG사 인증·승인 호출이 직렬로 수행되어 전체 지연의 61%를 차지한다. 이를 병렬화하고, 인증 토큰을 세션 단위 캐시로 저장하면 p95 0.9s 달성이 가능하다고 분석되었다.\n\n' +
      '현재 코드 리뷰 단계이며 v2.3.1 스프린트(7월 2주차)에 배포 예정이다. Slack 채널에서 배포 후 모니터링 경보 2건이 이 티켓으로 이관됐다.',
    facts: [
      { label: '현재 p95', value: '1.8s' },
      { label: '목표 p95', value: '0.9s' },
      { label: '배포 버전', value: 'v2.3.1' },
      { label: '원인', value: 'PG 직렬 호출 (지연의 61%)' },
    ],
    related: ['nb-deploy-thread', 'nb-api-spec'],
    citedBy: [],
  },
  {
    id: 'nb-api-spec',
    type: 'doc',
    sourceKind: 'drive',
    group: '제품',
    title: 'REST API 명세서 v3.1',
    origin: 'Confluence',
    modifiedAt: '6월 30일',
    fresh: { state: 'ok', label: '4일 전 확인' },
    summary:
      '외부 파트너 공개용 API 명세 3.1판. 인증이 API-Key에서 OAuth2로 전환됐고, ' +
      'v2 엔드포인트는 2026-10 지원 종료 예정. 파트너 공지 필요 항목이 4건.',
    topics: ['OAuth2 전환', 'v2 지원 종료', '파트너 공지 항목'],
    quote: 'v2 엔드포인트는 2026년 10월 31일부로 지원이 종료되며, 이후 요청은 410을 반환한다.',
    body:
      '외부 파트너 공개용 REST API 명세 3.1판. 가장 큰 변경은 인증 방식 전환(API-Key → OAuth2)이다.\n\n' +
      'v2 엔드포인트는 2026년 10월 31일부로 지원이 종료되며, 이후 요청은 410을 반환한다. 파트너사에 공지가 필요한 항목은 총 4건(인증 전환, v2 종료, Rate Limit 정책 변경, Webhook 포맷 변경)이다.\n\n' +
      '파트너 공지 초안 작성이 이번 스프린트 할 일에 포함돼 있다.',
    facts: [
      { label: '인증 전환', value: 'API-Key → OAuth2' },
      { label: 'v2 지원 종료', value: '2026-10-31' },
      { label: '공지 필요 항목', value: '4건' },
    ],
    related: ['nb-payment-ticket'],
    citedBy: [],
  },
  {
    id: 'nb-deploy-thread',
    type: 'chat',
    sourceKind: 'slack',
    group: '제품',
    title: 'v2.3.0 배포 완료 알림 및 후속 모니터링',
    origin: 'Slack',
    modifiedAt: '7월 2일',
    fresh: { state: 'ok', label: '오늘 확인' },
    summary:
      'v2.3.0 정식 배포 완료. 배포 후 24시간 에러율 0.02%로 안정. ' +
      '단, 결제 모듈 지연 알림이 2건 발생해 성능 개선 티켓과 연결됨.',
    topics: ['배포 안정성', '결제 지연 알림'],
    quote: '배포 후 모니터링에서 결제 응답 지연(p95>1.5s) 경보가 2건 발생, 해당 건은 PAY-412로 이관.',
    body:
      'v2.3.0 정식 배포가 7월 2일 오전 2시에 완료됐다. 배포 직후 무중단 배포 확인 절차를 거쳤다.\n\n' +
      '배포 후 24시간 모니터링 결과 전체 에러율 0.02%로 안정적이었다. 다만 결제 모듈에서 예외적인 지연 현상이 관측됐다.\n\n' +
      '배포 후 모니터링에서 결제 응답 지연(p95>1.5s) 경보가 2건 발생, 해당 건은 PAY-412로 이관. 개선 작업은 v2.3.1에서 진행 중이다.',
    facts: [
      { label: '배포 일시', value: '7/2 오전 2시' },
      { label: '24h 에러율', value: '0.02%' },
      { label: '결제 지연 경보', value: '2건 (PAY-412 이관)' },
    ],
    related: ['nb-payment-ticket'],
    citedBy: [],
  },
  {
    id: 'nb-team-okr',
    type: 'sheet',
    sourceKind: 'drive',
    group: '재무',
    title: '팀 OKR 2026-Q3.xlsx',
    origin: 'Google Drive',
    modifiedAt: '6월 28일',
    fresh: { state: 'stale', label: '10일 경과' },
    summary:
      '3분기 팀 OKR 시트. 핵심 목표 4개 — 매출 5억, 고객 만족 NPS ≥ 55, 인프라 비용 동결, 파트너 API v3 전환 완료. 현재 진행률이 30% 미만인 KR이 두 건.',
    topics: ['핵심 목표', '진행률 30% 미만 KR', 'NPS 목표'],
    quote: '인프라 비용 동결 KR의 현재 진행률은 12%이며 하반기 조치가 시급하다.',
    body:
      '3분기 팀 OKR 스프레드시트. 4개 목표(O1~O4)와 각 3개의 KR로 구성된다.\n\n' +
      'O1(매출 5억)은 현재 38% 달성, O2(NPS ≥ 55)는 현재 NPS 48로 목표에 못 미치고 있다. 인프라 비용 동결 KR의 현재 진행률은 12%이며 하반기 조치가 시급하다.\n\n' +
      '마지막 업데이트가 6월 28일이라 현재 상황이 반영되지 않았을 수 있다. 이번 주 금요일 OKR 리뷰 미팅 전에 갱신이 필요하다.',
    facts: [
      { label: 'O1 매출 달성률', value: '38% (목표: 5억)' },
      { label: 'O2 NPS', value: '현재 48 (목표: ≥ 55)' },
      { label: '인프라 비용 KR', value: '진행률 12%' },
      { label: '마지막 갱신', value: '6월 28일' },
    ],
    related: ['nb-quarter-report', 'nb-infra-quote-mail'],
    citedBy: [],
  },
  {
    id: 'nb-meeting-note-stale',
    type: 'doc',
    sourceKind: 'drive',
    group: '계약',
    title: '법무 검토 미팅 노트 (6/24)',
    origin: 'Confluence',
    modifiedAt: '6월 24일',
    fresh: { state: 'stale', label: '14일 경과' },
    summary:
      '6월 24일 법무팀 계약 검토 미팅 노트. B사 계약서 9조·12조 재협상 전략 논의. 다음 미팅은 7월 10일 예정. 노트 작성 후 14일이 경과한 구버전.',
    topics: ['9조 재협상 전략', '12조 대응 방안', '다음 미팅 일정'],
    quote: '9조는 배상 상한을 100분의 150으로 역제안하기로 했으며, 12조 공증 비용은 B사 부담을 요청한다.',
    body:
      '6월 24일 법무팀 미팅 노트. 참석자: 법무 팀장, 사업개발 2명, 외부 자문 변호사.\n\n' +
      '9조는 배상 상한을 100분의 150으로 역제안하기로 했으며, 12조 공증 비용은 B사 부담을 요청한다. 협상 레버리지로 B사가 요청한 SLA 조항 강화와 패키징하는 전략을 쓰기로 했다.\n\n' +
      '다음 협상 미팅은 7월 10일 예정이며, 역제안 문서는 7월 7일까지 법무에서 작성 완료 예정이었다. 이 노트는 14일 전 작성된 버전이므로 이후 진행 상황이 반영되지 않았을 수 있다.',
    facts: [
      { label: '역제안 배상 상한', value: '100분의 150' },
      { label: '12조 방침', value: '공증 비용 B사 부담 요청' },
      { label: '다음 미팅', value: '7월 10일' },
    ],
    related: ['nb-service-contract'],
    citedBy: [],
  },
  {
    id: 'nb-product-roadmap',
    type: 'slides',
    sourceKind: 'upload',
    group: '제품',
    title: '하반기 제품 로드맵 발표.pptx',
    origin: '업로드',
    modifiedAt: '6월 30일',
    fresh: { state: 'ok', label: '4일 전 확인' },
    summary:
      '하반기(Q3+Q4) 제품 로드맵 발표 슬라이드. 주요 이니셔티브: 결제 성능 개선, v2 API 전환 완료, 신규 대시보드 출시. v2 API 전환은 파트너 공지 일정에 의존.',
    topics: ['결제 개선 일정', 'v2 API 전환', '신규 대시보드'],
    quote: 'v2 API 전환 파트너 공지는 7월 15일까지 발송이 완료되어야 v3 전환이 Q3 내에 마무리된다.',
    body:
      '하반기 제품 로드맵 전체 발표 자료. 15페이지 슬라이드이며 이니셔티브 3개가 메인이다.\n\n' +
      'Initiative 1(결제 성능): v2.3.1에서 PG 병렬화 → Q3 내 p95 0.9s 달성. Initiative 2(API 전환): v2 API 지원 종료(10/31) 전 파트너 100% v3 전환 완료. v2 API 전환 파트너 공지는 7월 15일까지 발송이 완료되어야 v3 전환이 Q3 내에 마무리된다.\n\n' +
      'Initiative 3(신규 대시보드): Q4 출시. 파트너 공지 지연 시 Initiative 2가 Q4로 밀릴 위험 있음.',
    facts: [
      { label: '결제 개선 목표', value: 'Q3 내 p95 0.9s' },
      { label: '파트너 공지 마감', value: '7월 15일' },
      { label: 'v2 API 종료', value: '2026-10-31' },
    ],
    related: ['nb-api-spec', 'nb-payment-ticket'],
    citedBy: [],
  },
  {
    id: 'nb-sales-meeting-note',
    type: 'doc',
    sourceKind: 'drive',
    group: '재무',
    title: '영업팀 D사 초기 미팅 노트 (7/1)',
    origin: 'Google Drive',
    modifiedAt: '7월 1일',
    fresh: { state: 'ok', label: '1일 전 확인' },
    summary:
      'D사(엔터프라이즈 후보) 첫 미팅 노트. 연 계약 규모 예상 5천만 원. 결제 안정성과 API 파트너 연동 가능 여부가 주요 관심사. 다음 단계: 기술 데모 요청.',
    topics: ['계약 규모', '기술 데모 요청', 'API 연동 관심'],
    quote: 'D사 담당자는 결제 응답 시간이 SLA 기준 1초 이내임을 계약 전제 조건으로 언급했다.',
    body:
      'D사와의 첫 공식 미팅 노트(7월 1일). D사는 연 매출 500억 규모의 물류 SaaS 기업이며, 우리 플랫폼을 결제·정산 레이어로 검토 중이다.\n\n' +
      'D사 담당자는 결제 응답 시간이 SLA 기준 1초 이내임을 계약 전제 조건으로 언급했다. 현재 p95 1.8s는 이 조건에 미달하므로 v2.3.1 배포 결과가 선결 조건이 된다.\n\n' +
      '추가 관심사는 파트너 API v3 연동 지원 및 전용 기술 지원 SLA. 다음 단계로 기술 데모를 2주 내 요청했다.',
    facts: [
      { label: '예상 계약 규모', value: '연 5천만 원' },
      { label: 'SLA 요구', value: '결제 응답 1초 이내' },
      { label: '다음 단계', value: '기술 데모 (2주 내)' },
    ],
    related: ['nb-payment-ticket', 'nb-api-spec'],
    citedBy: [],
  },
  // --- "받은 재료" group: session artifacts, initially unconfirmed ---
  {
    id: 'nb-session-calc',
    type: 'session',
    sourceKind: 'session',
    group: '받은 재료',
    title: 'calc.py — 인프라 비용 시뮬레이션',
    origin: '세션 6c65148c',
    modifiedAt: '방금',
    fresh: { state: 'ok', label: '방금 생성' },
    summary:
      '세션에서 코워커 에이전트가 생성한 인프라 비용 시뮬레이션 스크립트. 트래픽 증가율 시나리오별 월별 비용을 계산하며 A클라우드 견적(320만/340만) 두 버전을 비교한다.',
    topics: ['트래픽 증가 시나리오', '비용 비교 (320 vs 340만)', '손익 분기점'],
    quote: '트래픽이 월 15% 증가 시, 320만 원 기준 초과 과금 상한(50만 원)은 4개월 이내에 도달한다.',
    body:
      '세션 6c65148c에서 인프라 비용 분석 요청 중 코워커 에이전트가 생성한 Python 스크립트 아티팩트이다.\n\n' +
      '스크립트는 트래픽 월간 증가율(5%/10%/15%/20%)을 파라미터로 받아 A클라우드 견적 두 버전(초기 340만, 재조정 320만)의 12개월 누적 비용을 계산한다.\n\n' +
      '트래픽이 월 15% 증가 시, 320만 원 기준 초과 과금 상한(50만 원)은 4개월 이내에 도달한다. 이 시나리오에서 연간 총비용 차이는 두 버전 간 약 240만 원이다.',
    facts: [
      { label: '생성 세션', value: '6c65148c' },
      { label: '비교 단가', value: '340만 vs 320만 원' },
      { label: '상한 도달 시점', value: '15% 성장 시 4개월' },
    ],
    related: ['nb-infra-quote-mail', 'nb-infra-quote-mail-v2'],
    citedBy: [],
    unconfirmed: true,
  },
  {
    id: 'nb-session-meeting-draft',
    type: 'session',
    sourceKind: 'session',
    group: '받은 재료',
    title: '회의록 초안 — 법무 협상 후속 미팅',
    origin: '세션 자동 생성',
    modifiedAt: '방금',
    fresh: { state: 'ok', label: '방금 생성' },
    summary:
      '법무 협상 후속 미팅(7/10) 준비를 위해 에이전트가 자동 생성한 회의록 초안. 기존 소스에서 추출한 협상 포인트와 역제안 논거가 포함돼 있다.',
    topics: ['협상 포인트 정리', '역제안 논거', '미팅 준비 체크리스트'],
    quote: '9조 역제안(150%)의 근거로 업계 표준 손해배상 상한(공정위 가이드라인 기준 100~150%)을 제시한다.',
    body:
      '7월 10일 법무 협상 후속 미팅을 위해 에이전트가 기존 소스들을 분석해 자동 생성한 회의록 초안이다.\n\n' +
      '9조 역제안(150%)의 근거로 업계 표준 손해배상 상한(공정위 가이드라인 기준 100~150%)을 제시한다. 12조 공증 비용 B사 부담 요청 시 SLA 강화 조항을 협상 카드로 제안한다.\n\n' +
      '이 초안은 법무 미팅 노트(6/24)와 계약서 초안을 합성해 작성됐다. 최종 확인 전 법무 팀장의 검토가 필요하다.',
    facts: [
      { label: '미팅 일자', value: '7월 10일' },
      { label: '9조 역제안', value: '100분의 150' },
      { label: '협상 카드', value: 'SLA 강화 조항' },
    ],
    related: ['nb-meeting-note-stale', 'nb-service-contract'],
    citedBy: [],
    unconfirmed: true,
  },
];

// ---------- Discover (AI source suggestions) ----------

export interface NbSuggestion {
  id: string;
  type: NbSourceType;
  title: string;
  origin: string;
  reason: string; // why the AI suggests it
}

export const DISCOVER_SUGGESTIONS: NbSuggestion[] = [
  {
    id: 'sg-cloud-pricing',
    type: 'link',
    title: 'A클라우드 공식 요금 정책 페이지',
    origin: 'web',
    reason: '견적 메일의 초과 과금 조항과 대조할 수 있는 공식 단가표',
  },
  {
    id: 'sg-standard-contract',
    type: 'link',
    title: '공정위 표준 서비스 공급 계약서 (2025 개정)',
    origin: 'web',
    reason: '계약서 9조 손해배상 상한의 통상 수준 비교 기준',
  },
  {
    id: 'sg-pg-benchmark',
    type: 'link',
    title: 'PG사별 결제 API 응답속도 벤치마크',
    origin: 'web',
    reason: '결제 성능 개선 목표(0.9s)의 업계 기준선 확인용',
  },
];

// ---------- Scripted answers ----------

export interface NbAnswerSeg {
  text: string;
  cites?: string[]; // source ids rendered as citation pills after this segment
  conflictWith?: {
    // if set: two citation sources have conflicting values — renders ⚠ pill
    srcA: string;      // source id
    valA: string;      // what srcA says
    srcB: string;      // source id
    valB: string;      // what srcB says
    hint: string;      // "최신 메일(7/01) 기준 유력" etc.
  };
}

interface Script {
  match: RegExp;
  needs: string[]; // source ids that make this script relevant
  segs: NbAnswerSeg[];
}

const SCRIPTS: Script[] = [
  {
    match: /실적|매출|2분기|분기/,
    needs: ['nb-quarter-report'],
    segs: [
      { text: '2분기 매출은 4.2억 원으로 전분기 대비 18% 성장했고, 신규 고객 32곳이 주요 동력이었습니다.', cites: ['nb-quarter-report'] },
      { text: ' 다만 클라우드 인프라 비용이 22% 늘어 영업이익률을 2.1%p 깎았습니다.', cites: ['nb-quarter-report'] },
      { text: ' 배포 채널 기록을 보면 v2.3.0 이후 서비스 자체는 안정적(에러율 0.02%)이라, 비용 쪽이 3분기 최우선 과제로 보입니다.', cites: ['nb-deploy-thread'] },
    ],
  },
  {
    // Conflict script: two mails quote different prices for the same vendor.
    match: /견적|인프라|비용|클라우드/,
    needs: ['nb-infra-quote-mail', 'nb-infra-quote-mail-v2'],
    segs: [
      {
        text: 'A클라우드 견적이 두 버전 존재합니다 — 최초 견적 340만 원과 당일 재조정 견적 320만 원.',
        cites: ['nb-infra-quote-mail', 'nb-infra-quote-mail-v2'],
        conflictWith: {
          srcA: 'nb-infra-quote-mail',
          valA: '"월 요금 340만 원 기준" — 최초 견적 메일',
          srcB: 'nb-infra-quote-mail-v2',
          valB: '"최종 조정 단가 320만 원" — 후속 재조정 메일',
          hint: '최신 메일(7/01 재조정) 기준 320만 원이 유력하나, 담당자 확인 필요',
        },
      },
      { text: ' 재조정 견적에는 초과 과금 상한(월 50만 원)이 추가됐습니다. 실적 보고서의 인프라 비용 증가 추세와 함께 시뮬레이션해 보는 것을 권합니다.', cites: ['nb-quarter-report', 'nb-infra-quote-mail-v2'] },
    ],
  },
  {
    match: /견적|인프라|비용|클라우드/,
    needs: ['nb-infra-quote-mail'],
    segs: [
      { text: 'A클라우드 견적은 월 340만 원(3년 약정 시 15% 할인)입니다.', cites: ['nb-infra-quote-mail'] },
      { text: ' 주의할 점 두 가지 — 트래픽 10TB 초과분에 GB당 120원이 별도 과금되고, 중도 해지 위약금 조항이 있습니다.', cites: ['nb-infra-quote-mail'] },
      { text: ' 실적 보고서 기준 인프라 비용이 이미 22% 증가 추세라, 초과 과금 조항은 실제 트래픽 전망과 함께 검토하는 게 안전합니다.', cites: ['nb-quarter-report'] },
    ],
  },
  {
    match: /계약|손해배상|9조|조항/,
    needs: ['nb-service-contract'],
    segs: [
      { text: 'B사 계약 초안에서 가장 주의할 부분은 9조입니다 — 손해배상 상한이 "최근 3개월 대금의 200%"로, 지난 표준계약보다 우리 쪽에 불리합니다.', cites: ['nb-service-contract'] },
      { text: ' 12조 데이터 반환 의무도 함께 수정됐으니 법무 검토 시 두 조항을 묶어서 보는 것을 권합니다.', cites: ['nb-service-contract'] },
    ],
  },
  {
    match: /성능|결제|응답|지연/,
    needs: ['nb-payment-ticket'],
    segs: [
      { text: '결제 API p95가 1.8s까지 악화됐고, 원인은 PG사 인증·승인 호출의 직렬 수행(전체 지연의 61%)입니다.', cites: ['nb-payment-ticket'] },
      { text: ' 병렬화 + 캐시로 0.9s를 목표하고 있으며 v2.3.1에 배포 예정입니다.', cites: ['nb-payment-ticket'] },
      { text: ' 실제로 v2.3.0 배포 후 모니터링에서 결제 지연 경보 2건이 이 티켓(PAY-412)으로 이관됐습니다.', cites: ['nb-deploy-thread'] },
    ],
  },
];

/** Build a scripted or generic answer from the current question + checked sources. */
export function answerFor(question: string, checked: NbSource[]): NbAnswerSeg[] {
  const ids = new Set(checked.map((s) => s.id));
  for (const sc of SCRIPTS) {
    if (sc.match.test(question) && sc.needs.every((n) => ids.has(n))) {
      // Drop citations that point outside the current grounding scope.
      return sc.segs.map((seg) => ({
        text: seg.text,
        cites: seg.cites?.filter((c) => ids.has(c)),
        conflictWith: seg.conflictWith,
      }));
    }
  }
  // Generic fallback: ground on up to two checked sources.
  const picks = checked.slice(0, 2);
  if (picks.length === 0) return [{ text: '선택된 소스가 없어 답변을 근거 지을 수 없어요.' }];
  const segs: NbAnswerSeg[] = [
    {
      text: `선택된 ${checked.length}개 소스를 기준으로 보면, ${picks[0].title.replace(/\.[a-z]+$/i, '')} 쪽 내용이 가장 직접적입니다 — ${picks[0].summary.split('.')[0]}.`,
      cites: [picks[0].id],
    },
  ];
  if (picks[1]) {
    segs.push({
      text: ` 함께 보면 좋은 맥락으로, ${picks[1].summary.split('.')[0]}.`,
      cites: [picks[1].id],
    });
  }
  segs.push({ text: ' 더 구체적으로 파고들 부분을 알려주시면 해당 대목을 인용해 드릴게요.' });
  return segs;
}

// ---------- Studio output palette ----------

export type NbStudioKind = 'report' | 'table' | 'mindmap' | 'slides';

export const STUDIO_PALETTE: { kind: NbStudioKind; icon: string; label: string }[] = [
  { kind: 'report', icon: '📋', label: '브리핑 리포트' },
  { kind: 'table', icon: '📊', label: '데이터 표' },
  { kind: 'mindmap', icon: '🗺️', label: '마인드맵' },
  { kind: 'slides', icon: '📽️', label: '슬라이드 개요' },
];

export function studioBody(kind: NbStudioKind, checked: NbSource[]): string {
  const names = checked.slice(0, 3).map((s) => s.title.split('.')[0]);
  switch (kind) {
    case 'report':
      return [
        '## 주간 브리핑 — 선택 소스 기준',
        '',
        '**1. 재무** · 2분기 매출 4.2억(+18%), 인프라 비용 +22%가 이익률 압박.',
        '**2. 계약 리스크** · B사 계약 9조 손해배상 상한(3개월 대금의 200%) 재협상 필요.',
        '**3. 기술** · 결제 p95 1.8s → 0.9s 개선 진행, v2.3.1 배포 예정.',
        '',
        `_근거: ${names.join(' · ')}_`,
      ].join('\n');
    case 'table':
      return [
        '| 항목 | 현재 | 목표/기한 | 출처 |',
        '|---|---|---|---|',
        '| 2분기 매출 | 4.2억 (+18%) | 3Q 5억 | 실적 보고서 |',
        '| 인프라 견적 | 월 320~340만 | 3년 약정 -15% | 견적 메일 |',
        '| 결제 p95 | 1.8s | 0.9s (v2.3.1) | Jira |',
        '| v2 API | 운영 중 | 10/31 종료 | 명세서 |',
      ].join('\n');
    case 'mindmap':
      return [
        '● 3분기 준비',
        '├─ 재무',
        '│  ├─ 매출 5억 목표',
        '│  └─ 인프라 비용 최적화 ⚠',
        '├─ 계약',
        '│  ├─ B사 9조 재협상',
        '│  └─ A클라우드 약정 검토',
        '└─ 제품',
        '   ├─ 결제 성능 v2.3.1',
        '   └─ v2 API 종료 공지',
      ].join('\n');
    case 'slides':
      return [
        '1. 표지 — 3분기 전략 브리핑',
        '2. 2분기 성과 하이라이트 (매출 +18%)',
        '3. 비용 구조: 인프라 22% 증가의 원인',
        '4. 계약 리스크 2건과 대응안',
        '5. 제품 로드맵: 결제 성능·API 전환',
        '6. 3분기 목표와 마일스톤',
      ].join('\n');
  }
}

// Source kind → display label for filter chips.
export const KIND_FILTER_LABELS: Record<NbSourceKind | 'all', string> = {
  all: '전체',
  drive: 'Drive',
  gmail: 'Gmail',
  slack: 'Slack',
  jira: 'Jira',
  upload: '업로드',
  session: '세션',
};
