# Workspace Hub — 다중소스 허브 UI 설계 (item 3)

- 날짜: 2026-07-03
- 대상: `app_v2/` `/workspace` 라우트 (현 씨앗 UI를 대체)
- 상태: 사용자 승인 완료 (브레인스토밍 세션, 시각 목업 3라운드)

## 목적

Workspace를 "파일 리스트"에서 **여러 소스를 근원으로 하는 문서·정보를 한곳에서 보고 관리하는 허브**로 탈바꿈한다. 이번 라운드의 충실도: **UI 셸 완성 + 로컬 파일 실동작 + 타소스는 목데이터로 UX 완전 시연**. 실제 외부 커넥터는 후속.

## 확정 결정 (사용자 선택)

| 결정 | 내용 |
|------|------|
| 레이아웃 | **소스 레일형(A) + 통합 '전체' 하이브리드** — 레일 최상단 '전체'가 통합 최근/검색을 흡수 |
| 소스 세트 | 로컬(실동작) + Google Drive, Amazon S3, Confluence, Jira, Gmail, Slack (목업 6종) |
| 소스 아이콘 | 각 소스의 **실제 브랜드 SVG 아이콘**을 번들로 내장 (외부 CDN 금지) |
| '전체' 행 규격 | **B안**: 18px 고정 아이콘 + **76px 고정폭 소스명 칼럼** + 제목(말줄임) + 우측 메타 — 모든 행 균일 |
| 채팅 연결 | 로컬 항목 "새 대화에서 쓰기" **실동작**(홈 컴포저 프리필), 목업 소스는 동일 버튼 + 안내 토스트 |

## 정보 구조 (IA)

```
Workspace (기존 앱 사이드바의 탭)
└── 소스 레일 (Workspace 내부 좌측, ~190px)
    ├── ⌂ 전체            ← 통합 최근순 목록 + 통합 검색 (기본 진입 뷰)
    ├── [파일]            로컬 · Google Drive · Amazon S3
    ├── [문서 · 티켓]      Confluence · Jira
    ├── [메시지]          Gmail · Slack
    └── ＋ 소스 추가       ← 목업 다이얼로그 (커넥터 로드맵 안내)
```

- 레일 항목: 브랜드 아이콘 + 소스명 + 우측 카운트. 활성 항목은 인디고 액센트.
- 메인 영역: 상단 툴바(검색 인풋 + 컨텍스트 액션[로컬/전체=업로드]) + 목록 + 우측 **디테일 패널**(~260px).
- 디테일 패널: 메타(소스 배지·크기·시각) + 미니 미리보기 + 액션 스택("새 대화에서 쓰기" 프라이머리 / 다운로드·삭제·원본 열기 서브). 파일의 전체 미리보기는 기존 `FilePreviewModal` 라이트박스 재사용(패널의 미리보기 클릭 시).

## 뷰 아키타입 3종 (소스들이 공유)

| 아키타입 | 소스 | 형태 |
|----------|------|------|
| **파일형** `files` | 로컬(실동작) · Drive · S3 | 브레드크럼 + 폴더 탐색, 이름/크기/수정일 칼럼 |
| **항목형** `items` | Confluence · Jira | 제목 + 상태/라벨 칩 + 담당/스페이스, 목록형 |
| **스레드형** `threads` | Gmail · Slack | 발신인/채널 + 제목/미리보기 한 줄, 상세는 말풍선 스레드 |

'전체' 뷰는 아키타입과 무관하게 위의 **B안 행 규격**으로 평탄화해 최신순 정렬.

## 프론트 아키텍처

**핵심 추상화 — `SourceProvider`** (`app_v2/src/workspace/` 신설 모듈):

```ts
interface SourceProvider {
  id: string;                    // 'local' | 'gdrive' | 's3' | 'confluence' | 'jira' | 'gmail' | 'slack'
  name: string;                  // 표시명 (i18n)
  icon: ComponentType;           // 번들 브랜드 SVG
  category: 'files' | 'docs' | 'messages';
  kind: 'files' | 'items' | 'threads';   // 뷰 아키타입 선택
  connected: boolean;            // 목업도 true (시연 목적) — attach 가능 여부와 별개
  attachable: boolean;           // 로컬만 true
  list(ctx: ListCtx): Promise<SourceEntry[]>;   // ctx: 폴더 경로 등
  recent(): Promise<SourceEntry[]>;             // '전체' 공급 (최대 ~20개)
  detail(id: string): Promise<SourceDetail>;
}
```

- `LocalProvider`: 기존 `api/workspace.ts`(webdav)의 어댑터 — list=listDirectory, detail=stat+getFileBlob, 업로드/삭제 위임.
- `MockProvider(fixture)`: 소스별 정적 한국어 픽스처(JSON) — 지연 시뮬레이션(수백 ms) 포함해 로딩 상태도 시연.
- **레지스트리 배열 하나**(`providers.ts`)가 레일 렌더·라우팅·'전체' 병합을 전부 구동. 실제 커넥터 도입 = provider 구현체 추가로 끝나는 구조.
- '전체' = `Promise.all(providers.map(p => p.recent()))` 병합 → 최신순 정렬 → 클라이언트 측 텍스트 필터(검색 인풋).
- 라우팅: `/workspace` = 전체, `/workspace/$sourceId` = 소스 뷰 (TanStack Router 파라미터, 레일 활성 상태 연동).

## 채팅 연결 ("새 대화에서 쓰기")

- **로컬(실동작)**: 디테일 패널의 프라이머리 액션 → `/`(홈 컴포저)로 navigate하며 첨부 참조를 **소형 모듈 store**(`stores/pendingAttachment.ts`, `stores/project.ts`와 같은 싱글턴 패턴)로 전달 → 컴포저에 **첨부 칩** 표시(파일명, 전송 전 제거 가능) → 전송 시 본문에 `[첨부 파일: /root/shared/<상대경로>]` 참조를 포함해 sendMessage. coworker sandbox는 workspace를 `/root/shared`(ro)로 마운트하므로 에이전트가 실제로 읽을 수 있다.
- **목업 소스**: 동일 버튼 노출, 클릭 시 토스트 "커넥터 연결 후 사용할 수 있어요" — 액션의 존재 자체가 제품 방향을 시연.
- 첨부 칩이 있을 때 컴포저의 에이전트 피커는 **coworker를 기본 선택**한다(sandbox 마운트가 있는 유일한 타입이므로). 사용자가 deep_research로 바꾸는 것은 막지 않는다.

## 시각/디자인

- app_v2 디자인 시스템(이식된 cowork-design-system + theme.css 인디고 팔레트) 준수. 레일·행·패널 모두 기존 토큰 사용.
- 브랜드 아이콘: simple-icons 계열 SVG를 `app_v2/src/workspace/icons/`에 정적 포함, 18px 슬롯 규격.
- 목업 데이터는 자연스러운 한국어 업무 콘텐츠(회의록, 계약, 배포 논의, 이슈 티켓 등)로 채워 데모 체감을 높인다.

## 범위 제외 (명시)

- 실제 외부 커넥터·OAuth·토큰 관리 (후속 라운드)
- 서버측 통합 검색/인덱싱 (전체 검색은 클라이언트 필터)
- backend_v2 변경 일절 없음 (순수 프론트)
- 목업 소스에 대한 쓰기 작업(업로드·삭제는 로컬만)
- 페이지네이션/무한스크롤 (픽스처 규모에선 불필요)

## 테스트

- providers 레지스트리: 병합 정렬(recent), 카테고리 그룹핑 단위 테스트
- 뷰 아키타입 3종: 픽스처 렌더 스모크 (testing-library)
- LocalProvider 어댑터: webdav 모킹 위 list/detail 매핑
- attach 프리필 흐름: 디테일 패널 → navigate + 칩 → sendMessage 본문에 참조 포함
- 기존 39개 테스트 유지 — 참고: 현 workspace 라우트를 직접 커버하는 테스트는 없음(유일한 workspace 테스트는 webdav API 래퍼 단위 테스트로 이번 변경과 무관). 라우트 교체로 깨지는 기존 테스트는 없고, 새 UI 테스트는 전부 net-new

## 성공 기준

1. `/workspace` 진입 시 '전체' 뷰: 7개 소스 항목이 균일 행으로 최신순 병합
2. 레일에서 각 소스 진입: 아키타입별 뷰가 콘텐츠 문법에 맞게 렌더 (로컬은 실제 파일)
3. 로컬 파일 → "새 대화에서 쓰기" → 컴포저 칩 → 전송 → coworker가 파일 내용 기반 응답 (라이브 검증)
4. 목업 소스의 동일 액션 → 안내 토스트
5. lint/test/build green
