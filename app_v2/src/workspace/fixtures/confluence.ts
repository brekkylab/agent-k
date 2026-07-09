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
      '이 문서는 제품 전반에서 사용되는 디자인 시스템의 컴포넌트 사용 원칙과 토큰 체계를 설명합니다. 색상, 타이포그래피, 간격, 아이콘 사용법이 포함되어 있으며 v2에서는 다크 모드 지원이 추가되었습니다.',
    externalUrl: '#',
  },
  'confluence-page-meeting-0625': {
    entry: entries[1],
    bodyPreview:
      '참석: 김민준, 이수연, 박지훈, 최아름\n안건 1: Q3 로드맵 우선순위 조정\n안건 2: 인프라 마이그레이션 일정 재검토\n결론: 마이그레이션은 8월로 연기, 디자인 시스템 v2 배포 7월 2주차 확정.',
    externalUrl: '#',
  },
  'confluence-page-api-spec': {
    entry: entries[2],
    bodyPreview:
      'v3.1에서 추가된 엔드포인트: POST /workspaces/{id}/members, GET /analytics/usage. 인증은 Bearer 토큰 방식 유지. 페이지네이션은 cursor 기반으로 변경됨.',
    externalUrl: '#',
  },
  'confluence-page-onboarding': {
    entry: entries[3],
    bodyPreview:
      '첫째 주: 팀 소개 및 시스템 접근 권한 설정. 둘째 주: 코드베이스 파악 및 shadowing. 셋째 주: 첫 번째 작은 태스크 독립 수행. 문의는 HR 채널(#hr-support) 이용.',
    externalUrl: '#',
  },
  'confluence-page-security-policy': {
    entry: entries[4],
    bodyPreview:
      '2026년 개정 주요 변경 사항: 비밀번호 최소 길이 12자로 상향, MFA 전 직원 필수 적용, 외부 SaaS 연동 시 보안 검토 프로세스 의무화.',
    externalUrl: '#',
  },
  'confluence-page-arch-decision': {
    entry: entries[5],
    bodyPreview:
      '배경: 모놀리스 배포 병목 현상으로 인한 출시 지연. 결정: 핵심 도메인(사용자, 결제, 알림)을 독립 서비스로 분리. 트레이드오프: 운영 복잡도 증가 감수.',
    externalUrl: '#',
  },
  'confluence-page-release-checklist': {
    entry: entries[6],
    bodyPreview:
      '배포 전: DB 마이그레이션 검토, 롤백 스크립트 준비, 스테이징 스모크 테스트 통과 확인. 배포 후: 에러율 모니터링 30분, Slack #alerts 채널 감시.',
    externalUrl: '#',
  },
  'confluence-page-customer-journey': {
    entry: entries[7],
    bodyPreview:
      '인지 → 탐색 → 가입 → 활성화 → 유지 단계별 고객 감정 곡선 분석. 주요 이탈 지점: 가입 후 3일 이내 첫 핵심 기능 미사용. 개선 방향: 온보딩 투어 강화.',
    externalUrl: '#',
  },
  'confluence-page-infra-diagram': {
    entry: entries[8],
    bodyPreview:
      'AWS 기반 3-티어 아키텍처. 프론트엔드: CloudFront + S3. 백엔드: ECS Fargate (Auto Scaling). 데이터베이스: RDS Aurora + ElastiCache Redis. DR 리전: ap-southeast-1.',
    externalUrl: '#',
  },
  'confluence-page-sprint-retro': {
    entry: entries[9],
    bodyPreview:
      'Keep: 일일 스탠드업 15분 유지, PR 리뷰 당일 완료 문화. Problem: QA 환경 불안정으로 테스트 지연. Try: QA 환경 Docker Compose화, 자동화 테스트 커버리지 80% 목표.',
    externalUrl: '#',
  },
};
