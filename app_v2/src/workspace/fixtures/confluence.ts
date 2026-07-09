import type { SourceEntry, SourceDetail } from '../types';

export const entries: SourceEntry[] = [
  {
    id: 'confluence-page-design-system',
    sourceId: 'confluence',
    title: '디자인 시스템 가이드라인 v2',
    subtitle: 'DESIGN 스페이스',
    kind: 'item',
    modifiedAt: '2026-07-02T10:30:00.000Z',
  },
  {
    id: 'confluence-page-meeting-0625',
    sourceId: 'confluence',
    title: '팀 주간 회의록 2026-06-25',
    subtitle: 'TEAM 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-25T18:00:00.000Z',
  },
  {
    id: 'confluence-page-api-spec',
    sourceId: 'confluence',
    title: 'REST API 명세서 v3.1',
    subtitle: 'DEV 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-30T09:00:00.000Z',
  },
  {
    id: 'confluence-page-onboarding',
    sourceId: 'confluence',
    title: '신규 입사자 온보딩 가이드',
    subtitle: 'HR 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-24T11:00:00.000Z',
  },
  {
    id: 'confluence-page-security-policy',
    sourceId: 'confluence',
    title: '정보보안 정책 2026년 개정판',
    subtitle: 'SECURITY 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-26T14:00:00.000Z',
  },
  {
    id: 'confluence-page-arch-decision',
    sourceId: 'confluence',
    title: '마이크로서비스 전환 결정 기록 (ADR-012)',
    subtitle: 'ARCH 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-21T13:00:00.000Z',
  },
  {
    id: 'confluence-page-release-checklist',
    sourceId: 'confluence',
    title: '배포 체크리스트 템플릿',
    subtitle: 'DEV 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-19T10:00:00.000Z',
  },
  {
    id: 'confluence-page-customer-journey',
    sourceId: 'confluence',
    title: '고객 여정 지도 (Customer Journey Map)',
    subtitle: 'PRODUCT 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-22T15:30:00.000Z',
  },
  {
    id: 'confluence-page-infra-diagram',
    sourceId: 'confluence',
    title: '클라우드 인프라 아키텍처 다이어그램',
    subtitle: 'INFRA 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-23T16:00:00.000Z',
  },
  {
    id: 'confluence-page-sprint-retro',
    sourceId: 'confluence',
    title: 'Sprint 42 회고 노트',
    subtitle: 'TEAM 스페이스',
    kind: 'item',
    modifiedAt: '2026-06-20T17:00:00.000Z',
  },
];

export const details: Record<string, SourceDetail> = {
  'confluence-page-design-system': {
    entry: entries[0],
    bodyPreview:
      '이 문서는 제품 전반에서 사용되는 디자인 시스템의 컴포넌트 사용 원칙과 토큰 체계를 설명합니다.\n\n## v2 scope\n- 색상, 타이포그래피, 간격, 아이콘 사용법을 제품 전반에 통일합니다.\n- 다크 모드 토큰과 semantic color alias가 추가되었습니다.\n\n## Review checklist\n- [x] Button / input state 정리\n- [x] Workspace row density 기준 반영\n- [ ] Chart color token QA',
    externalUrl: '#',
  },
  'confluence-page-meeting-0625': {
    entry: entries[1],
    bodyPreview:
      '참석: 김민준, 이수연, 박지훈, 최아름\n\n## Agenda\n- Q3 로드맵 우선순위 조정\n- 인프라 마이그레이션 일정 재검토\n\n## Decisions\n- [x] 마이그레이션은 8월로 연기\n- [x] 디자인 시스템 v2 배포는 7월 2주차로 확정',
    externalUrl: '#',
  },
  'confluence-page-api-spec': {
    entry: entries[2],
    bodyPreview:
      'REST API v3.1 변경 명세입니다.\n\n## Added endpoints\n- POST /workspaces/{id}/members\n- GET /analytics/usage\n\n## Compatibility notes\n- 인증은 Bearer 토큰 방식을 유지합니다.\n- 페이지네이션은 offset에서 cursor 기반으로 변경됩니다.\n- [ ] SDK example 업데이트 필요',
    externalUrl: '#',
  },
  'confluence-page-onboarding': {
    entry: entries[3],
    bodyPreview:
      '신규 입사자 온보딩 가이드입니다.\n\n## First three weeks\n- 첫째 주: 팀 소개 및 시스템 접근 권한 설정\n- 둘째 주: 코드베이스 파악 및 shadowing\n- 셋째 주: 첫 번째 작은 태스크 독립 수행\n\n## Support\n- [ ] HR 채널 #hr-support 안내\n- [ ] Workspace source 권한 확인',
    externalUrl: '#',
  },
  'confluence-page-security-policy': {
    entry: entries[4],
    bodyPreview:
      '2026년 정보보안 정책 개정판입니다.\n\n## Major changes\n- 비밀번호 최소 길이를 12자로 상향합니다.\n- MFA를 전 직원 필수로 적용합니다.\n- 외부 SaaS 연동 시 보안 검토 프로세스를 의무화합니다.\n\n## Rollout\n- [x] 정책 초안 승인\n- [ ] 팀별 예외 요청 수집',
    externalUrl: '#',
  },
  'confluence-page-arch-decision': {
    entry: entries[5],
    bodyPreview:
      'ADR-012: 마이크로서비스 전환 결정 기록입니다.\n\n## Context\n- 모놀리스 배포 병목 현상으로 출시 지연이 반복됩니다.\n- 사용자, 결제, 알림 도메인의 변경 주기가 다릅니다.\n\n## Decision\n- 핵심 도메인을 독립 서비스로 분리합니다.\n- 운영 복잡도 증가는 observability 개선으로 상쇄합니다.\n- [x] Architecture review 승인',
    externalUrl: '#',
  },
  'confluence-page-release-checklist': {
    entry: entries[6],
    bodyPreview:
      '배포 체크리스트 템플릿입니다.\n\n## Before deploy\n- [ ] DB 마이그레이션 검토\n- [ ] 롤백 스크립트 준비\n- [ ] 스테이징 스모크 테스트 통과 확인\n\n## After deploy\n- 에러율 30분 모니터링\n- Slack #alerts 채널 감시',
    externalUrl: '#',
  },
  'confluence-page-customer-journey': {
    entry: entries[7],
    bodyPreview:
      '고객 여정 지도 분석 문서입니다.\n\n## Stages\n- 인지 → 탐색 → 가입 → 활성화 → 유지\n- 주요 이탈 지점은 가입 후 3일 이내 첫 핵심 기능 미사용입니다.\n\n## Opportunities\n- 온보딩 투어 강화\n- 첫 workspace source 연결을 더 짧게 만들기\n- [ ] 고객 세그먼트별 friction 비교',
    externalUrl: '#',
  },
  'confluence-page-infra-diagram': {
    entry: entries[8],
    bodyPreview:
      '클라우드 인프라 아키텍처 다이어그램 설명입니다.\n\n## Components\n- Frontend: CloudFront + S3\n- Backend: ECS Fargate with Auto Scaling\n- Data: RDS Aurora + ElastiCache Redis\n\n## Resilience\n- DR 리전은 ap-southeast-1입니다.\n- [ ] Failover runbook 최신화',
    externalUrl: '#',
  },
  'confluence-page-sprint-retro': {
    entry: entries[9],
    bodyPreview:
      'Sprint 42 회고 노트입니다.\n\n## Keep\n- 일일 스탠드업 15분 유지\n- PR 리뷰 당일 완료 문화\n\n## Problem\n- QA 환경 불안정으로 테스트가 지연되었습니다.\n\n## Try\n- [ ] QA 환경 Docker Compose화\n- [ ] 자동화 테스트 커버리지 80% 목표',
    externalUrl: '#',
  },
};
