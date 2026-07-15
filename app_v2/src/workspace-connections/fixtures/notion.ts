import type { SourceEntry, SourceDetail } from '../types';

export const entries: SourceEntry[] = [
  {
    id: 'notion-page-company-os',
    sourceId: 'notion',
    title: 'Company OS',
    subtitle: 'Workspace',
    kind: 'page',
    modifiedAt: '2026-07-03T12:30:00.000Z',
    parentId: null,
    emoji: '🏢',
  },
  {
    id: 'notion-page-hiring-plan',
    sourceId: 'notion',
    title: 'Hiring Plan',
    subtitle: 'People',
    kind: 'page',
    modifiedAt: '2026-06-28T09:00:00.000Z',
    parentId: 'notion-page-company-os',
    emoji: '👥',
  },
  {
    id: 'notion-page-q3-product-strategy',
    sourceId: 'notion',
    title: 'Q3 Product Strategy',
    subtitle: 'Product',
    kind: 'page',
    modifiedAt: '2026-07-05T10:00:00.000Z',
    parentId: 'notion-page-company-os',
    emoji: '🧭',
  },
  {
    id: 'notion-page-workspace-source-grounding',
    sourceId: 'notion',
    title: 'Workspace Source Grounding',
    subtitle: 'Product',
    kind: 'page',
    modifiedAt: '2026-07-06T11:45:00.000Z',
    parentId: 'notion-page-q3-product-strategy',
    emoji: '📚',
  },
  {
    id: 'notion-page-connector-notes',
    sourceId: 'notion',
    title: 'Notion Connector Notes',
    subtitle: 'Integration',
    kind: 'page',
    modifiedAt: '2026-07-04T16:20:00.000Z',
    parentId: 'notion-page-q3-product-strategy',
    emoji: '🔌',
  },
  {
    id: 'notion-page-customer-research',
    sourceId: 'notion',
    title: 'Customer Research',
    subtitle: 'Research',
    kind: 'page',
    modifiedAt: '2026-07-02T15:10:00.000Z',
    parentId: null,
    emoji: '🎙️',
  },
  {
    id: 'notion-page-interview-notes',
    sourceId: 'notion',
    title: 'Interview Notes',
    subtitle: 'Research',
    kind: 'page',
    modifiedAt: '2026-07-01T13:00:00.000Z',
    parentId: 'notion-page-customer-research',
    emoji: '📝',
  },
  {
    id: 'notion-page-objection-library',
    sourceId: 'notion',
    title: 'Objection Library',
    subtitle: 'Sales',
    kind: 'page',
    modifiedAt: '2026-06-29T17:00:00.000Z',
    parentId: 'notion-page-customer-research',
    emoji: '💬',
  },
  {
    id: 'notion-page-engineering',
    sourceId: 'notion',
    title: 'Engineering',
    subtitle: 'Team',
    kind: 'page',
    modifiedAt: '2026-06-30T10:30:00.000Z',
    parentId: null,
    emoji: '🛠️',
  },
  {
    id: 'notion-page-api-decisions',
    sourceId: 'notion',
    title: 'API Decisions',
    subtitle: 'Architecture',
    kind: 'page',
    modifiedAt: '2026-06-27T14:30:00.000Z',
    parentId: 'notion-page-engineering',
    emoji: '🧱',
  },
];

export const details: Record<string, SourceDetail> = {
  'notion-page-company-os': {
    entry: entries[0],
    bodyPreview:
      'Company OS는 팀의 운영 원칙, 의사결정 기록, 채용 계획, 제품 전략을 한 곳에서 연결하는 최상위 페이지입니다.\n\n## Current operating loop\n- Weekly product review는 Customer Research와 Q3 Product Strategy를 함께 확인합니다.\n- 승인된 결정은 Engineering / API Decisions로 옮겨 실행 owner를 붙입니다.\n\n## Open upkeep\n- [x] Source grounding 원칙 정리\n- [ ] Hiring Plan과 Q3 roadmap dependency 연결',
    externalUrl: '#',
  },
  'notion-page-hiring-plan': {
    entry: entries[1],
    bodyPreview:
      'Q3 채용 계획: Product Engineer 2명, Customer Success 1명.\n\n## Interview loop\n- Recruiter screen: role fit과 availability 확인\n- Product pairing: 실제 workspace source flow를 읽고 작은 개선안 제안\n- Team debrief: trial task는 90분 안에 끝나야 합니다.\n\n## Status\n- [x] Product Engineer scorecard 초안\n- [ ] Customer Success interview kit',
    externalUrl: '#',
  },
  'notion-page-q3-product-strategy': {
    entry: entries[2],
    bodyPreview:
      'Q3 제품 전략은 workspace가 source bucket이 아니라 성장하는 작업 맥락으로 보이게 만드는 데 집중합니다.\n\n## Pillars\n- Source grounding: 답변이 어느 source에서 왔는지 추적 가능해야 합니다.\n- Collection record: 반복되는 지식을 재사용 가능한 record로 가꿉니다.\n- Reusable context: 세션 밖에서도 workspace가 이어져야 합니다.\n\n## Decisions\n- [x] Notion은 page tree를 유지해서 보여준다\n- [ ] Collection record 승인 플로우와 연결',
    externalUrl: '#',
  },
  'notion-page-workspace-source-grounding': {
    entry: entries[3],
    bodyPreview:
      'Workspace Source Grounding은 Google Drive, Gmail, Notion 페이지를 같은 질문 맥락에서 참조하되, 각 답변이 어떤 source에서 왔는지 추적 가능해야 한다는 원칙을 정리합니다.\n\n## Principle\n- 답변은 source별 provenance chip을 가져야 합니다.\n- 같은 사실이 여러 source에 있을 때 최신성과 권한 경계를 함께 보여줍니다.\n- Notion page는 하위 페이지 구조를 잃지 않고 참조되어야 합니다.\n\n## Acceptance notes\n- [x] SourceRail에는 provider만 둔다\n- [x] Notion page tree는 source view 내부에서 펼친다\n- [ ] Workspace collection에 심을 때 원문 page 위치를 유지한다',
    externalUrl: '#',
  },
  'notion-page-connector-notes': {
    entry: entries[4],
    bodyPreview:
      'Notion connector mock은 실제 OAuth 없이 page tree, child page, page body preview만 보여줍니다.\n\n## Mock scope\n- Frontend registry에만 provider를 추가합니다.\n- 실제 sync, token storage, permission mirror는 범위 밖입니다.\n- Page block graph는 bodyPreview fixture로 대체합니다.\n\n## Later\n- [ ] OAuth connection card\n- [ ] Block-level provenance',
    externalUrl: '#',
  },
  'notion-page-customer-research': {
    entry: entries[5],
    bodyPreview:
      'Customer Research는 고객 인터뷰, 반대 의견, 구매 기준을 모아 제품 방향과 세일즈 메시지를 함께 다듬기 위한 공간입니다.\n\n## Current questions\n- 고객은 source 연결보다 answer trust를 먼저 묻습니다.\n- Workspace가 어떤 순간에 “살아있는 맥락”으로 느껴지는지 확인합니다.\n\n## Follow-ups\n- [x] Interview Notes 정리\n- [ ] Objection Library를 sales enablement와 연결',
    externalUrl: '#',
  },
  'notion-page-interview-notes': {
    entry: entries[6],
    bodyPreview:
      '인터뷰 노트 요약: 사용자는 흩어진 문서보다 “지금 이 대화가 어떤 근거 위에서 진행되는지”를 더 빠르게 확인하고 싶어합니다.\n\n## Quotes to preserve\n- “답은 좋은데 어디서 온 말인지 바로 보고 싶다.”\n- “검색 결과보다 지금 작업 중인 맥락이 더 중요하다.”\n\n## Signals\n- [x] 오른쪽 detail에서 원문 확인 필요\n- [ ] Collection으로 보낼 때 원문 위치 표시',
    externalUrl: '#',
  },
  'notion-page-objection-library': {
    entry: entries[7],
    bodyPreview:
      '자주 나오는 반대 의견과 대응 메모입니다.\n\n## Objections\n- 기존 Drive 검색으로 충분하지 않은가\n- AI가 오래된 문서를 인용하지 않는가\n- 팀별 권한 경계를 어떻게 보존하는가\n\n## Response angles\n- [x] 최신성 표시\n- [ ] 권한 경계 설명 카드',
    externalUrl: '#',
  },
  'notion-page-engineering': {
    entry: entries[8],
    bodyPreview:
      'Engineering page는 API 결정, 배포 체크리스트, connector 구현 메모를 하위 페이지로 묶어 개발 의사결정의 provenance를 남깁니다.\n\n## Working agreements\n- Mock source는 frontend-only로 둡니다.\n- 실제 connector는 backend token model이 정리된 뒤 시작합니다.\n\n## Checklist\n- [x] Provider registry 확장\n- [ ] Block-level fixture schema 검토',
    externalUrl: '#',
  },
  'notion-page-api-decisions': {
    entry: entries[9],
    bodyPreview:
      'API Decisions: mock source는 frontend registry에만 추가하고, 실제 connector config와 token storage는 backend 설계가 확정될 때까지 도입하지 않습니다.\n\n## Decision\n- SourceProvider.kind에 pages를 추가합니다.\n- SourceEntry는 parentId와 emoji를 optional로 가집니다.\n- 실제 Notion API block schema는 아직 도입하지 않습니다.\n\n## Rejected for mock\n- [x] Backend connector table 추가\n- [x] OAuth token storage 추가',
    externalUrl: '#',
  },
};
