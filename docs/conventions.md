# Project Conventions

이 문서는 서버(backend)와 클라이언트(frontend)의 코딩 컨벤션을 정리합니다.
코드 수정 시 기존 컨벤션을 유지해야 하며, 형태를 변경하기 전에 반드시 확인을 받아야 합니다.

---

## Backend (Rust / Actix-web)

### Naming

| 대상 | 규칙 | 예시 |
|------|------|------|
| 파일/모듈 | snake_case | `handlers.rs`, `agent/spec.rs` |
| 함수/변수 | snake_case | `create_agent`, `provider_profile_id` |
| 구조체/Enum | PascalCase | `Agent`, `ProviderProfile`, `SessionStatus` |
| 상수 | SCREAMING_SNAKE_CASE | `DEFAULT_DATABASE_URL` |

### 프로젝트 구조

```
backend/src/
├── main.rs              # 서버 진입점, 라우트 설정
├── handlers.rs          # HTTP 핸들러 + OpenAPI 문서 + 라우트 설정
├── models.rs            # 도메인 모델 + Request/Response DTO
├── state.rs             # 앱 상태 (AppState)
├── agent/
│   ├── mod.rs
│   └── spec.rs          # AgentSpec, AgentProvider 타입
└── repository/
    ├── mod.rs           # Repository 트레잇 + 팩토리
    ├── sqlite.rs        # SQLite 구현
    └── postgres.rs      # PostgreSQL 구현 (스텁)
```

### API 규칙

| 항목 | 규칙 |
|------|------|
| URL | kebab-case, 리소스 중심 (`/provider-profiles/{id}`) |
| Method | REST 표준 (GET=조회, POST=생성, PUT=수정, DELETE=삭제) |
| 응답 코드 | 200(OK), 201(Created), 204(No Content), 400(Bad Request), 404(Not Found), 409(Conflict) |
| Body | JSON, `Content-Type: application/json` |
| 에러 | `ErrorResponse { error: String }` 통일 |
| 문서화 | utoipa의 `#[utoipa::path(...)]` 매크로로 OpenAPI 자동 생성 |

### 타입 패턴

- 도메인 모델: `#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]`
- Request DTO: `#[derive(Debug, Deserialize, ToSchema)]` + `#[serde(deny_unknown_fields)]`
- Response DTO: 민감 정보 redact (`redact_agent_provider()` — API 키 제거)
- ID: `uuid::Uuid` (v4)
- 시간: `chrono::DateTime<Utc>`
- Enum 직렬화: `#[serde(rename_all = "snake_case")]` 또는 `#[serde(tag = "type", rename_all = "lowercase")]`

### 아키텍처 패턴

- **Repository 패턴**: `Repository` 트레잇 → `SqliteRepository` / `PostgresRepository` 구현
- **환경 변수 기반 설정**: `DATABASE_URL`, `BIND_ADDR`, `OPENAI_API_KEY` 등
- **요청 검증**: 핸들러 내에서 빈 문자열 체크, 비즈니스 로직 검증
- **에러 처리 체인**: `RepositoryError` → `repository_error_response()` → `json_error()`

### 테스트

- 모듈 내 `#[cfg(test)] mod tests` 방식
- `#[actix_web::test]` 매크로
- 헬퍼 함수로 테스트 데이터 생성 (`create_agent()`, `create_provider_profile()`)
- `TempDir`로 임시 DB 사용

---

## Frontend (TypeScript / Next.js)

### Naming

| 대상 | 규칙 | 예시 |
|------|------|------|
| 파일 | kebab-case | `mock-runtime-provider.tsx`, `knowledge-editor.tsx` |
| 컴포넌트 | PascalCase | `DocumentList`, `KnowledgeEditor`, `ChatView` |
| 함수/변수/훅 | camelCase | `useAppStore`, `handleFiles`, `isExpanded` |
| Props 인터페이스 | PascalCase + Props 접미사 | `ChatViewProps`, `KnowledgeEditorProps` |
| 이벤트 핸들러 | on + 동사 | `onToggleKnowledgePanel`, `onDragOver` |

### 프로젝트 구조

```
frontend/
├── app/                          # Next.js App Router
│   ├── layout.tsx               # 루트 레이아웃 (Server Component)
│   ├── page.tsx                 # / → /chat redirect
│   ├── chat/page.tsx
│   ├── sources/page.tsx
│   ├── knowledge/page.tsx
│   └── settings/page.tsx
├── components/
│   ├── ui/                      # shadcn/ui 프리미티브 (수정 최소화)
│   ├── layout/                  # 레이아웃 (sidebar, theme)
│   ├── assistant-ui/            # assistant-ui 래퍼
│   ├── chat/                    # 채팅 관련
│   ├── sources/                 # Source 업로드/관리
│   ├── knowledge/               # Knowledge 관련
│   └── settings/                # Provider 프로필 관리
├── lib/
│   ├── types.ts                 # 도메인 타입 정의 (중앙)
│   ├── store.ts                 # Zustand 스토어 (단일)
│   ├── api.ts                   # Backend fetch 래퍼 + API 함수
│   ├── constants.ts             # Provider별 모델/설정 상수
│   └── utils.ts                 # cn() 등 유틸리티
```

### 컴포넌트 규칙

| 항목 | 규칙 |
|------|------|
| 컴포넌트 | 함수형 컴포넌트 (`function Component()` 또는 `const Component: FC`) |
| "use client" | 인터랙티브 컴포넌트에만 명시. 레이아웃은 Server Component |
| Props | `interface XxxProps` 정의 후 구조분해 할당 |
| 내보내기 | named export (`export function`, `export const`) |

### 상태 관리 (Zustand)

- **단일 스토어**: `useAppStore` (`lib/store.ts`)
- **셀렉터 패턴**: `const activeSessionId = useAppStore((s) => s.activeSessionId)`
- **비-React 접근**: `useAppStore.getState().someAction()` (이벤트 핸들러, 어댑터 내부)
- **액션 네이밍**: 동사+명사 (`setActiveSession`, `updateSessionKnowledge`, `bumpSessionListVersion`)
- **초기화**: Backend API에서 데이터 로드, persist 미사용

### 스타일링

| 항목 | 규칙 |
|------|------|
| 프레임워크 | Tailwind CSS 4 |
| 조건부 클래스 | `cn()` 유틸리티 (clsx + tailwind-merge) |
| 컴포넌트 변형 | CVA (class-variance-authority) |
| 색상 토큰 | 시맨틱 토큰 사용 (`bg-background`, `text-foreground`, `bg-primary`) |
| 반응형 | `md:`, `sm:`, `lg:` breakpoint prefix |
| 다크 모드 | next-themes, `attribute="class"`, 기본 light |

### Import 순서

```typescript
// 1. React/Next.js
import { useState, useEffect } from "react";
import { useRouter } from "next/navigation";

// 2. 외부 라이브러리
import { ChevronDown, X } from "lucide-react";
import { ThreadPrimitive } from "@assistant-ui/react";

// 3. 타입 import
import type { ChatMessage } from "@/lib/types";

// 4. 내부 유틸/스토어 (@/ alias)
import { cn } from "@/lib/utils";
import { useAppStore } from "@/lib/store";

// 5. 컴포넌트
import { Button } from "@/components/ui/button";
```

### TypeScript 규칙

- **interface**: Props, State 정의에 사용
- **type**: import type 문법으로 타입만 가져올 때
- **도메인 타입 중앙화**: `lib/types.ts`에 모든 도메인 인터페이스 정의
- **strict mode**: 활성화 (`tsconfig.json`)
- **경로 alias**: `@/*` → 프로젝트 루트 (`tsconfig.json`)

---

## 공통 규칙

### Git

- **브랜치**: `main` (기본), `backend`, `frontend` 등 기능별 브랜치
- **커밋 메시지**: Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`)
- **Co-Author**: 자동 생성 코드에 Co-Authored-By 태그

### 코드 변경 시 주의

- 기존 네이밍 컨벤션을 변경하지 말 것 (예: snake_case → camelCase 전환 금지)
- 새 파일 추가 시 기존 디렉토리 구조를 따를 것
- API URL 패턴을 변경하지 말 것 (kebab-case 유지)
- shadcn/ui 컴포넌트(`components/ui/`)는 직접 수정 최소화
- **형태를 해치는 변경이 필요한 경우 반드시 사전 확인 필요**
