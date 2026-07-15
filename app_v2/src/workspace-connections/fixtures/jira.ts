import type { SourceEntry, SourceDetail } from '../types';

export const entries: SourceEntry[] = [
  {
    id: 'jira-issue-DEV-201',
    sourceId: 'jira',
    title: '결제 모듈 성능 개선 (응답 시간 50% 단축)',
    subtitle: '[DEV-201] 진행 중',
    kind: 'item',
    modifiedAt: '2026-07-02T11:00:00.000Z',
  },
  {
    id: 'jira-issue-DEV-198',
    sourceId: 'jira',
    title: '회원가입 이메일 인증 버그 수정',
    subtitle: '[DEV-198] 완료',
    kind: 'item',
    modifiedAt: '2026-07-01T16:00:00.000Z',
  },
  {
    id: 'jira-issue-DEV-195',
    sourceId: 'jira',
    title: '대시보드 차트 실시간 업데이트 기능',
    subtitle: '[DEV-195] 코드 리뷰',
    kind: 'item',
    modifiedAt: '2026-06-30T13:00:00.000Z',
  },
  {
    id: 'jira-issue-OPS-45',
    sourceId: 'jira',
    title: 'RDS 인스턴스 스케일 업 (db.r6g.xlarge)',
    subtitle: '[OPS-45] 완료',
    kind: 'item',
    modifiedAt: '2026-06-28T09:00:00.000Z',
  },
  {
    id: 'jira-issue-DEV-192',
    sourceId: 'jira',
    title: 'Slack 알림 웹훅 연동 구현',
    subtitle: '[DEV-192] 테스트 중',
    kind: 'item',
    modifiedAt: '2026-06-27T14:30:00.000Z',
  },
  {
    id: 'jira-issue-SEC-12',
    sourceId: 'jira',
    title: 'API 엔드포인트 Rate Limiting 적용',
    subtitle: '[SEC-12] 진행 중',
    kind: 'item',
    modifiedAt: '2026-06-26T10:00:00.000Z',
  },
  {
    id: 'jira-issue-DEV-188',
    sourceId: 'jira',
    title: '파일 업로드 최대 크기 제한 버그',
    subtitle: '[DEV-188] 완료',
    kind: 'item',
    modifiedAt: '2026-06-24T15:00:00.000Z',
  },
  {
    id: 'jira-issue-PROD-33',
    sourceId: 'jira',
    title: 'v2.3.0 릴리스 태스크 트래킹',
    subtitle: '[PROD-33] 진행 중',
    kind: 'item',
    modifiedAt: '2026-06-23T09:30:00.000Z',
  },
  {
    id: 'jira-issue-DEV-185',
    sourceId: 'jira',
    title: '다국어 지원(i18n) 초기 설정',
    subtitle: '[DEV-185] 완료',
    kind: 'item',
    modifiedAt: '2026-06-21T11:00:00.000Z',
  },
  {
    id: 'jira-issue-UX-27',
    sourceId: 'jira',
    title: '모바일 반응형 레이아웃 개선',
    subtitle: '[UX-27] 백로그',
    kind: 'item',
    modifiedAt: '2026-06-20T10:00:00.000Z',
  },
];

export const details: Record<string, SourceDetail> = {
  'jira-issue-DEV-201': {
    entry: entries[0],
    bodyPreview:
      '결제 API 평균 응답 시간이 1.2초로 SLA 기준인 600ms를 초과하고 있습니다.\n\n## Scope\n- N+1 쿼리 패턴을 배치 조회로 교체합니다.\n- Redis 캐싱 레이어를 추가합니다.\n- Checkout summary endpoint를 별도 측정합니다.\n\n## Status\n- [x] Slow query trace 수집\n- [ ] Batch loader 적용\n- [ ] 부하 테스트 재실행',
    externalUrl: '#',
  },
  'jira-issue-DEV-198': {
    entry: entries[1],
    bodyPreview:
      '이메일 인증 링크가 만료 전에도 "링크 만료" 오류를 반환하는 버그입니다.\n\n## Root cause\n- JWT iat 시간대가 UTC와 KST 사이에서 불일치했습니다.\n- 만료 검증 함수가 local offset을 중복 적용했습니다.\n\n## Resolution\n- [x] 서버 전체 UTC 통일\n- [x] 토큰 생성 로직 수정\n- [x] Regression test 추가',
    externalUrl: '#',
  },
  'jira-issue-DEV-195': {
    entry: entries[2],
    bodyPreview:
      'WebSocket 기반 실시간 차트 업데이트 구현 이슈입니다.\n\n## Current state\n- PR #204 리뷰 대기 중입니다.\n- 테스트 커버리지는 78%입니다.\n- 리뷰어는 김민준, 이수연입니다.\n\n## Remaining\n- [ ] reconnect 상태 UI 확인\n- [ ] chart diff throttling 검토',
    externalUrl: '#',
  },
  'jira-issue-OPS-45': {
    entry: entries[3],
    bodyPreview:
      'DB CPU가 지속적으로 70%를 초과하여 RDS 인스턴스를 스케일 업했습니다.\n\n## Result\n- 다운타임 0분을 달성했습니다. Multi-AZ Failover 사용.\n- 비용은 월 $340 증가합니다.\n- 모니터링 알람 임계값을 재조정했습니다.\n\n## Follow-up\n- [x] 스케일 업 완료\n- [ ] 비용 리포트에 반영',
    externalUrl: '#',
  },
  'jira-issue-DEV-192': {
    entry: entries[4],
    bodyPreview:
      'Slack Incoming Webhook 연동 구현 이슈입니다.\n\n## Events\n- 배포 완료 알림\n- 에러 알람\n- 일일 요약 리포트\n\n## Status\n- 현재 스테이징 환경에서 통합 테스트 진행 중입니다.\n- [ ] retry policy 검증\n- [ ] 알림 noise threshold 조정',
    externalUrl: '#',
  },
  'jira-issue-SEC-12': {
    entry: entries[5],
    bodyPreview:
      'API 엔드포인트 Rate Limiting 적용 이슈입니다.\n\n## Limits\n- 공개 API: IP당 분당 60회\n- 인증된 API: 사용자당 분당 300회\n- 구현은 nginx + Redis 기반입니다.\n\n## Approval\n- [ ] 보안팀 승인\n- [ ] 429 response copy 확정',
    externalUrl: '#',
  },
  'jira-issue-DEV-188': {
    entry: entries[6],
    bodyPreview:
      '50MB 이상 파일 업로드 시 백엔드 연결이 끊기는 문제입니다.\n\n## Cause\n- Nginx client_max_body_size가 설정되지 않았습니다.\n\n## Fix\n- [x] client_max_body_size를 100MB로 설정\n- [x] 프론트엔드 진행 표시 개선\n- [x] 80MB fixture upload smoke 통과',
    externalUrl: '#',
  },
  'jira-issue-PROD-33': {
    entry: entries[7],
    bodyPreview:
      'v2.3.0 릴리스 태스크 트래킹입니다.\n\n## Checklist\n- [x] 기능 개발\n- [x] QA\n- [ ] 문서화\n- [ ] 스테이징 최종 확인\n- [ ] 프로덕션 배포\n\n## Target\n- 릴리스 목표일은 2026-07-07입니다.',
    externalUrl: '#',
  },
  'jira-issue-DEV-185': {
    entry: entries[8],
    bodyPreview:
      'react-i18next 기반 한국어/영어 지원 초기 설정 이슈입니다.\n\n## Completed\n- 번역 파일 구조 수립\n- 날짜/숫자 포맷 로케일 처리\n- 언어 전환 UI 연결\n\n## Later\n- [ ] 일본어 추가 가능성 검토\n- [ ] missing key report 자동화',
    externalUrl: '#',
  },
  'jira-issue-UX-27': {
    entry: entries[9],
    bodyPreview:
      '모바일 반응형 레이아웃 개선 백로그입니다.\n\n## Issues\n- 375px 환경에서 테이블 레이아웃이 깨집니다.\n- 버튼 클릭 영역이 좁습니다.\n- 관련 컴포넌트는 DataTable, ActionBar입니다.\n\n## Next\n- [ ] Q3 디자인 스프린트에 포함\n- [ ] desktop regression screenshot 확보',
    externalUrl: '#',
  },
};
