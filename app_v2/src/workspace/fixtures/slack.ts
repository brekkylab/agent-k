import type { SourceEntry, SourceDetail } from '../types';

export const entries: SourceEntry[] = [
  {
    id: 'slack-thread-deploy-alert',
    sourceId: 'slack',
    title: 'v2.3.0 배포 완료 알림 및 후속 모니터링',
    subtitle: '#dev-alerts',
    kind: 'thread',
    modifiedAt: '2026-07-02T07:45:00.000Z',
  },
  {
    id: 'slack-thread-incident-storage',
    sourceId: 'slack',
    title: '[인시던트] 스토리지 연결 오류 (해결 완료)',
    subtitle: '#incidents',
    kind: 'thread',
    modifiedAt: '2026-07-01T15:30:00.000Z',
  },
  {
    id: 'slack-thread-design-review',
    sourceId: 'slack',
    title: '디자인 시스템 v2 컴포넌트 리뷰 요청',
    subtitle: '#design',
    kind: 'thread',
    modifiedAt: '2026-06-30T14:00:00.000Z',
  },
  {
    id: 'slack-thread-q3-planning',
    sourceId: 'slack',
    title: 'Q3 로드맵 킥오프 준비 논의',
    subtitle: '#product',
    kind: 'thread',
    modifiedAt: '2026-06-29T11:00:00.000Z',
  },
  {
    id: 'slack-thread-code-review',
    sourceId: 'slack',
    title: 'PR #204 리뷰 요청 — 대시보드 실시간 업데이트',
    subtitle: '#code-review',
    kind: 'thread',
    modifiedAt: '2026-06-28T16:30:00.000Z',
  },
  {
    id: 'slack-thread-team-lunch',
    sourceId: 'slack',
    title: '7월 팀 런치 메뉴 투표',
    subtitle: '#random',
    kind: 'thread',
    modifiedAt: '2026-06-27T12:00:00.000Z',
  },
  {
    id: 'slack-thread-security-update',
    sourceId: 'slack',
    title: 'Rate Limiting 정책 구현 논의',
    subtitle: '#security',
    kind: 'thread',
    modifiedAt: '2026-06-26T10:30:00.000Z',
  },
  {
    id: 'slack-thread-onboarding-help',
    sourceId: 'slack',
    title: '신규 입사자 정윤서 온보딩 지원',
    subtitle: '#general',
    kind: 'thread',
    modifiedAt: '2026-06-24T09:00:00.000Z',
  },
  {
    id: 'slack-thread-api-feedback',
    sourceId: 'slack',
    title: 'v3.1 API 피드백 및 개선 제안',
    subtitle: '#backend',
    kind: 'thread',
    modifiedAt: '2026-06-22T14:00:00.000Z',
  },
  {
    id: 'slack-thread-standup-0620',
    sourceId: 'slack',
    title: '주간 스탠드업 요약 2026-06-20',
    subtitle: '#standup',
    kind: 'thread',
    modifiedAt: '2026-06-20T10:15:00.000Z',
  },
];

export const details: Record<string, SourceDetail> = {
  'slack-thread-deploy-alert': {
    entry: entries[0],
    bodyPreview:
      '배포봇: ✅ v2.3.0이 프로덕션에 성공적으로 배포되었습니다. (07:40 KST)\n박지훈: 에러율 확인 중입니다. 현재 0.02%로 정상 범위입니다.\n김민준: Sentry 알람도 조용하네요. 이번 배포 깔끔합니다 👍\n이수연: 30분 후 최종 모니터링 결과 공유하겠습니다.',
    externalUrl: '#',
  },
  'slack-thread-incident-storage': {
    entry: entries[1],
    bodyPreview:
      '모니터링봇: 🔴 ALERT: 스토리지 서비스 응답 없음 감지 (15:12)\n최아름: 확인 중입니다. S3 연결 타임아웃 에러 발생.\n최아름: ECS 태스크 재시작으로 임시 조치. 고객 영향 3분 추정.\n최아름: 🟢 해결 완료 (15:28). 원인: AWS us-east-1 일시 장애. 포스트모텀 내일까지 작성 예정.',
    externalUrl: '#',
  },
  'slack-thread-design-review': {
    entry: entries[2],
    bodyPreview:
      '정디자인: 디자인 시스템 v2 주요 컴포넌트 Figma 링크 공유합니다. Button, Input, Modal, Toast 업데이트됐습니다.\n김민준: Button의 disabled 상태 색상이 접근성 기준 미달 같은데요? 확인 부탁드립니다.\n정디자인: 맞습니다. 4.2:1 → 4.5:1로 수정하겠습니다. 내일까지 업데이트할게요.',
    externalUrl: '#',
  },
  'slack-thread-q3-planning': {
    entry: entries[3],
    bodyPreview:
      '이PM: Q3 로드맵 킥오프를 다음 주 월요일에 진행하려 합니다. 각 팀 OKR 초안을 금요일까지 공유해 주세요.\n박지훈: 개발팀 초안은 목요일까지 드리겠습니다. 이번 분기 핵심은 성능 개선과 모바일 대응입니다.\n이PM: 좋습니다. 마케팅팀도 같은 일정으로 요청드렸습니다.',
    externalUrl: '#',
  },
  'slack-thread-code-review': {
    entry: entries[4],
    bodyPreview:
      '이수연: PR #204 리뷰 부탁드립니다. WebSocket으로 대시보드 실시간 업데이트 구현했습니다. 테스트 커버리지 78%.\n김민준: 전반적으로 좋은데 useEffect 의존성 배열에 누락이 있어요. Line 142 확인해보세요.\n이수연: 수정했습니다! 감사해요. 재리뷰 부탁드립니다.',
    externalUrl: '#',
  },
  'slack-thread-team-lunch': {
    entry: entries[5],
    bodyPreview:
      '박지훈: 이번 달 팀 런치 투표합니다! 1) 한식 (삼겹살 회식) 2) 일식 (스시 오마카세) 3) 양식 (이탈리안 파스타)\n최아름: 2번이요! 오마카세 가고 싶었어요 🍣\n김민준: 저도 2번!\n이수연: 1번 한식이요 ㅎㅎ\n박지훈: 일식으로 결정! 이번 주 금요일 6시 예약할게요.',
    externalUrl: '#',
  },
  'slack-thread-security-update': {
    entry: entries[6],
    bodyPreview:
      '최보안: Rate Limiting 구현 방향 논의합니다. nginx + Redis sliding window 방식 제안드립니다.\n박지훈: Redis 의존성 추가보다 nginx 내장 limit_req_zone으로 시작하는 게 어떨까요? 단순하게.\n최보안: 좋은 의견입니다. 단, 분산 환경에서 정확도는 Redis가 낫습니다. 2단계로 나눠서 진행하겠습니다.',
    externalUrl: '#',
  },
  'slack-thread-onboarding-help': {
    entry: entries[7],
    bodyPreview:
      '이PM: 오늘부터 정윤서 님이 합류했습니다. #general에서 인사 나눠주세요!\n정윤서: 안녕하세요! 프론트엔드 개발자로 합류하게 됐습니다. 잘 부탁드립니다 😊\n김민준: 환영합니다! 궁금한 점 있으면 DM 주세요.\n이수연: 저도 온보딩 도움 드릴게요. 내일 오전에 코드베이스 설명 드리겠습니다.',
    externalUrl: '#',
  },
  'slack-thread-api-feedback': {
    entry: entries[8],
    bodyPreview:
      '이수연: v3.1 API 사용해봤는데 cursor 페이지네이션이 훨씬 편하네요. 그런데 에러 응답 포맷이 일부 엔드포인트에서 다릅니다.\n박지훈: 맞아요, /analytics 쪽은 아직 구 포맷이에요. DEV-205로 티켓 생성했습니다.\n이수연: 감사합니다! 그리고 POST /workspaces/{id}/members 응답에 멤버 전체 목록 포함해주면 좋겠어요.',
    externalUrl: '#',
  },
  'slack-thread-standup-0620': {
    entry: entries[9],
    bodyPreview:
      '스탠드업봇: 📋 주간 스탠드업 요약 (2026-06-20)\n김민준: [완료] DEV-185 i18n 초기 설정 / [진행] DEV-195 대시보드 차트\n이수연: [완료] OPS-45 DB 스케일업 완료 확인 / [진행] 모니터링 대시보드 개선\n박지훈: [완료] DEV-188 파일 업로드 버그 수정 / [진행] DEV-201 결제 성능 개선\n이PM: 이번 주 목표: v2.3.0 QA 완료.',
    externalUrl: '#',
  },
};
