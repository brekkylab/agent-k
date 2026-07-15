import type { SourceEntry, SourceDetail } from '../types';

export const entries: SourceEntry[] = [
  {
    id: 'gmail-thread-q2-review',
    sourceId: 'gmail',
    title: '[긴급] 2분기 실적 검토 미팅 일정 조율',
    subtitle: '박대표 <ceo@example.com>',
    kind: 'thread',
    modifiedAt: '2026-07-02T08:30:00.000Z',
  },
  {
    id: 'gmail-thread-vendor-proposal',
    sourceId: 'gmail',
    title: 'Re: 클라우드 인프라 견적서 검토 요청',
    subtitle: '최영업 <sales@cloudvendor.com>',
    kind: 'thread',
    modifiedAt: '2026-07-01T14:00:00.000Z',
  },
  {
    id: 'gmail-thread-security-alert',
    sourceId: 'gmail',
    title: '[보안 알림] 비정상적인 로그인 시도 감지',
    subtitle: 'Security Team <security-noreply@example.com>',
    kind: 'thread',
    modifiedAt: '2026-07-01T09:15:00.000Z',
  },
  {
    id: 'gmail-thread-partner-intro',
    sourceId: 'gmail',
    title: '파트너십 제안 — AI 스타트업 B사',
    subtitle: '이대표 <ceo@bcompany.ai>',
    kind: 'thread',
    modifiedAt: '2026-06-30T11:00:00.000Z',
  },
  {
    id: 'gmail-thread-hr-benefit',
    sourceId: 'gmail',
    title: '2026년 하반기 복지 제도 변경 안내',
    subtitle: 'HR팀 <hr@example.com>',
    kind: 'thread',
    modifiedAt: '2026-06-28T10:00:00.000Z',
  },
  {
    id: 'gmail-thread-customer-complaint',
    sourceId: 'gmail',
    title: 'Re: 서비스 오류 관련 불편 접수',
    subtitle: '고객지원 <support@example.com>',
    kind: 'thread',
    modifiedAt: '2026-06-27T16:00:00.000Z',
  },
  {
    id: 'gmail-thread-legal-contract',
    sourceId: 'gmail',
    title: '[법무] NDA 계약서 최종 검토 완료',
    subtitle: '법무팀 <legal@example.com>',
    kind: 'thread',
    modifiedAt: '2026-06-26T13:30:00.000Z',
  },
  {
    id: 'gmail-thread-finance-report',
    sourceId: 'gmail',
    title: '6월 재무 현황 보고',
    subtitle: '재무팀 <finance@example.com>',
    kind: 'thread',
    modifiedAt: '2026-06-25T09:00:00.000Z',
  },
  {
    id: 'gmail-thread-conference-invite',
    sourceId: 'gmail',
    title: 'AWS Summit Seoul 2026 초청장',
    subtitle: 'AWS Events <events@aws.com>',
    kind: 'thread',
    modifiedAt: '2026-06-23T12:00:00.000Z',
  },
  {
    id: 'gmail-thread-newsletter',
    sourceId: 'gmail',
    title: '7월 업계 트렌드 뉴스레터',
    subtitle: 'TechBrief <newsletter@techbrief.io>',
    kind: 'thread',
    modifiedAt: '2026-06-20T08:00:00.000Z',
  },
];

export const details: Record<string, SourceDetail> = {
  'gmail-thread-q2-review': {
    entry: entries[0],
    bodyPreview:
      '박대표: 2분기 실적 최종 수치가 나왔습니다. 이번 주 내로 경영진 검토 미팅을 잡아야 할 것 같습니다. 가능한 일정 공유 부탁드립니다.\n나: 목요일 오후 2시 또는 금요일 오전 10시가 가능합니다. 슬라이드는 수요일까지 준비하겠습니다.\n박대표: 목요일 2시로 확정하겠습니다. 참석자 캘린더 초대 보내주세요.',
    externalUrl: '#',
  },
  'gmail-thread-vendor-proposal': {
    entry: entries[1],
    bodyPreview:
      '최영업: 지난번 미팅에서 논의한 클라우드 인프라 전환 견적서를 첨부합니다. 3년 약정 시 20% 추가 할인 적용 가능합니다.\n나: 검토 후 다음 주 초에 답변 드리겠습니다. 기술팀과 내부 검토 필요합니다.\n최영업: 이해합니다. 추가 문의사항 있으시면 언제든 연락 주세요.',
    externalUrl: '#',
  },
  'gmail-thread-security-alert': {
    entry: entries[2],
    bodyPreview:
      'Security Team: 귀하의 계정에 대한 비정상적인 로그인 시도가 감지되었습니다. IP: 203.0.113.42 (Singapore). 본인이 시도한 것이 아니라면 즉시 비밀번호를 변경해 주세요.\n나: 해당 접속 시도는 본인이 아닙니다. 비밀번호 변경 및 MFA 재설정 완료했습니다.',
    externalUrl: '#',
  },
  'gmail-thread-partner-intro': {
    entry: entries[3],
    bodyPreview:
      '이대표: 안녕하세요. B사는 생성형 AI 기반 문서 자동화 솔루션을 개발하고 있습니다. 귀사의 워크플로우와 시너지가 있을 것 같아 파트너십을 제안드립니다. 미팅 가능하실까요?\n나: 관심 있습니다. 간단한 소개 자료를 먼저 공유해 주시면 검토 후 일정을 잡겠습니다.',
    externalUrl: '#',
  },
  'gmail-thread-hr-benefit': {
    entry: entries[4],
    bodyPreview:
      'HR팀: 2026년 하반기부터 적용되는 복지 제도 변경 사항을 안내드립니다. 주요 변경: 1) 재택근무 주 3일 → 주 4일 허용, 2) 자기계발비 연 200만원으로 상향, 3) 건강검진 전액 지원. 자세한 내용은 사내 포털 확인 바랍니다.',
    externalUrl: '#',
  },
  'gmail-thread-customer-complaint': {
    entry: entries[5],
    bodyPreview:
      '고객지원: 2026-06-26 오후 3시경 파일 업로드 기능이 작동하지 않았다는 고객 불편 접수가 3건 있었습니다. 개발팀 확인 요청드립니다.\n나: 해당 시간대 서버 로그 확인 결과 스토리지 연결 일시 오류 발생이 확인됐습니다. 현재는 정상화됐으며, 고객께 개별 사과 메일 발송 예정입니다.',
    externalUrl: '#',
  },
  'gmail-thread-legal-contract': {
    entry: entries[6],
    bodyPreview:
      '법무팀: A사와의 NDA 계약서 최종 검토 완료했습니다. 3조 2항 "기밀 유지 기간" 5년→3년으로 수정 협의 완료. 대표이사 서명 후 반환 요청드립니다.\n나: 확인했습니다. 오늘 중으로 서명하겠습니다.',
    externalUrl: '#',
  },
  'gmail-thread-finance-report': {
    entry: entries[7],
    bodyPreview:
      '재무팀: 6월 재무 현황 보고드립니다. 매출: 전월 대비 +12%, 영업이익률: 18.3%, 현금 보유: 42억원. 상세 내역은 첨부 파일 참조 바랍니다. Q3 예산 계획 회의는 7월 15일로 예정되어 있습니다.',
    externalUrl: '#',
  },
  'gmail-thread-conference-invite': {
    entry: entries[8],
    bodyPreview:
      'AWS Events: AWS Summit Seoul 2026에 귀하를 초청합니다. 일시: 2026년 8월 22일(목)~23일(금). 장소: COEX 그랜드볼룸. 생성형 AI, 서버리스, 보안 등 100개 이상의 세션이 준비되어 있습니다. 무료 등록 마감: 8월 1일.',
    externalUrl: '#',
  },
  'gmail-thread-newsletter': {
    entry: entries[9],
    bodyPreview:
      'TechBrief: 이번 달 주요 트렌드: 1) 엣지 AI 칩 시장 급성장 (+340% YoY), 2) 양자컴퓨팅 상용화 첫 사례 등장, 3) 오픈소스 LLM 성능이 독점 모델에 근접. 전체 기사는 웹사이트에서 확인하세요.',
    externalUrl: '#',
  },
};
