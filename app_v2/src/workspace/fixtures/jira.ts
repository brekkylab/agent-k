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
      '현재 결제 API 평균 응답 시간이 1.2초로 SLA 기준(600ms) 초과 중. N+1 쿼리 패턴을 배치 조회로 교체하고 Redis 캐싱 레이어 추가 예정. 담당: 박지훈. 완료 목표: 2026-07-10.',
    externalUrl: '#',
  },
  'jira-issue-DEV-198': {
    entry: entries[1],
    bodyPreview:
      '이메일 인증 링크가 만료 전에도 "링크 만료" 오류를 반환하는 버그. 원인: JWT iat 시간대 불일치(UTC vs KST). 수정: 서버 전체 UTC 통일 + 토큰 생성 로직 수정 완료.',
    externalUrl: '#',
  },
  'jira-issue-DEV-195': {
    entry: entries[2],
    bodyPreview:
      'WebSocket 기반 실시간 차트 업데이트 구현. 현재 PR #204 리뷰 대기 중. 테스트 커버리지 78%. 리뷰어: 김민준, 이수연.',
    externalUrl: '#',
  },
  'jira-issue-OPS-45': {
    entry: entries[3],
    bodyPreview:
      'DB CPU 지속 70% 초과로 인한 스케일 업 완료. 다운타임 0분 달성 (Multi-AZ Failover). 비용 월 $340 증가. 모니터링 알람 임계값 재조정 완료.',
    externalUrl: '#',
  },
  'jira-issue-DEV-192': {
    entry: entries[4],
    bodyPreview:
      'Slack Incoming Webhook으로 배포 완료, 에러 알람, 일일 요약 리포트 발송 구현. 현재 스테이징 환경에서 통합 테스트 진행 중. 예상 완료: 2026-07-04.',
    externalUrl: '#',
  },
  'jira-issue-SEC-12': {
    entry: entries[5],
    bodyPreview:
      '공개 API: IP당 분당 60회, 인증된 API: 사용자당 분당 300회 제한 적용 예정. nginx + Redis 기반 구현. 보안팀 승인 대기 중.',
    externalUrl: '#',
  },
  'jira-issue-DEV-188': {
    entry: entries[6],
    bodyPreview:
      '50MB 이상 파일 업로드 시 백엔드 연결이 끊기는 문제. 원인: Nginx client_max_body_size 미설정. 수정: 100MB로 설정 + 프론트엔드 진행 표시 개선 완료.',
    externalUrl: '#',
  },
  'jira-issue-PROD-33': {
    entry: entries[7],
    bodyPreview:
      'v2.3.0 릴리스 체크리스트: [완료] 기능 개발, [완료] QA, [진행 중] 문서화, [대기] 스테이징 최종 확인, [대기] 프로덕션 배포. 릴리스 목표: 2026-07-07.',
    externalUrl: '#',
  },
  'jira-issue-DEV-185': {
    entry: entries[8],
    bodyPreview:
      'react-i18next 기반 한국어/영어 지원 초기 설정 완료. 번역 파일 구조 수립, 날짜/숫자 포맷 로케일 처리 포함. 향후 일본어 추가 가능한 구조.',
    externalUrl: '#',
  },
  'jira-issue-UX-27': {
    entry: entries[9],
    bodyPreview:
      '현재 모바일(375px) 환경에서 테이블 레이아웃 깨짐, 버튼 클릭 영역 협소 이슈 존재. Q3 디자인 스프린트에서 다룰 예정. 관련 컴포넌트: DataTable, ActionBar.',
    externalUrl: '#',
  },
};
