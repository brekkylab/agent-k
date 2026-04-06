# Project Conventions

이 문서는 프로젝트 전체의 아키텍처, 코딩 컨벤션, 패턴을 정리합니다.
코드 수정 시 기존 컨벤션을 유지해야 하며, 형태를 변경하기 전에 반드시 확인을 받아야 합니다.

---

## 전체 아키텍처

### 요청 흐름

```
[Frontend]  →  [Backend API]  →  [Service]  →  [Repository (SQLite)]
     ↑              ↓                ↓
     │          [AppState]     [chat-agent]
     │              ↓                ↓
     └─── SSE ← [런타임 캐시]   [ailoy Runtime]
                                     ↓
                              [Speedwagon 서브에이전트]
```

- **Frontend**: Next.js App Router. `assistant-ui`로 채팅 UI, `lib/api.ts`로 Backend 통신
- **Backend**: Actix-web HTTP 서버. 핸들러 → 서비스 → 레포지토리 3계층
- **chat-agent**: ailoy 런타임 래퍼 크레이트. ChatAgent, 도구(tools), Speedwagon 서브에이전트
- **AppState**: DB 연결, 업로드 경로, 세션별 ChatAgent 런타임 캐시를 관리

### 레이어별 책임

| 레이어 | 위치 | 역할 | 하지 않는 것 |
|--------|------|------|-------------|
| Handlers | `handlers.rs` | 라우팅, 입력 추출, service 위임, 응답 직렬화 | DB 직접 조회, 데이터 변환/매핑 |
| Services | `services/` | 비즈니스 로직, 여러 repository 호출 조합, 런타임 관리 | HTTP 관심사 (StatusCode, HttpResponse) |
| Repository | `repository/` | DB CRUD, SQL 쿼리, 트랜잭션 | 비즈니스 판단, 외부 서비스 호출 |
| Models | `models/` | 타입 정의 (Domain/Request/Response), 타입 간 변환(`From` impl) | 로직, DB 접근, 외부 호출 |
| chat-agent | `chat-agent/` | ailoy 런타임 생성, 스트리밍 실행, 도구 정의, 히스토리 관리 | DB 접근, HTTP 관심사 |

### 핸들러 경량화 원칙

핸들러는 **라우팅 + service 위임 + 응답 반환**만 담당한다.
DB를 두 번 이상 조회하거나 데이터 변환 로직이 5줄 이상이면 `services/` 함수로 추출한다.

```rust
// Good: 핸들러는 위임만
async fn get_session(...) -> HttpResponse {
    match session_service::get_session_detail(&state, id).await {
        Ok(Some(detail)) => HttpResponse::Ok().json(detail),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "not found"),
        Err(error) => error.error_response(),
    }
}
```

`ResponseError` 트레이트를 사용하려면 `use actix_web::ResponseError;` import 필요.

### 런타임 캐시

- `AppState.runtime_cache`: 세션 ID → `Arc<TokioMutex<ChatAgent>>` 매핑
- Agent/Provider/Speedwagon 변경 시 `invalidate_session_runtime()`으로 캐시 무효화
- 캐시 무효화 시 DB에서 최근 20턴(user/assistant만) 복원하여 새 런타임에 주입
- tool_call/tool_result 메시지는 복원되지 않음 (assistant 응답에 맥락 포함)

### 시스템 프롬프트 동적 조립

`prompt.rs::build_system_prompt()`에서 4단 레이어로 조립:

1. **Base Prompt** — 항상 포함 (identity, response_style, honesty)
2. **User Instructions** — Agent.spec.instruction이 있을 때만 (`<user_instructions>`)
3. **Dynamic Context** — Speedwagon KB 목록이 있을 때만 (`<available_knowledge_bases>`)
4. **Constraint Reminder** — 항상 마지막 (`<critical_reminder>`)

DB에는 raw user text만 저장하고, 조립된 프롬프트는 런타임 메모리에만 존재한다.

---

## Backend (Rust / Actix-web)

### 프로젝트 구조

```
backend/src/
├── main.rs              # 서버 진입점, 라우트/CORS 설정
├── handlers.rs          # HTTP 핸들러 + OpenAPI 문서 (utoipa)
├── prompt.rs            # 4단 레이어 시스템 프롬프트 동적 조립
├── state.rs             # AppState (DB, 런타임 캐시, 업로드 디렉토리)
├── models/              # 도메인 모델 + Request/Response DTO
│   ├── mod.rs           # pub use로 전체 re-export
│   ├── agent.rs         # Agent, AgentResponse, CreateAgentRequest, UpdateAgentRequest
│   ├── provider.rs      # ProviderProfile, ProviderProfileResponse, Create/UpdateProviderProfileRequest
│   ├── session.rs       # Session, SessionMessage, SessionToolCall,
│   │                    # SessionResponse, SessionDetailResponse, SessionMessageResponse,
│   │                    # Create/UpdateSessionRequest, AddSessionMessageRequest
│   ├── source.rs        # Source, SourceResponse, SourceType
│   ├── speedwagon.rs    # Speedwagon, SpeedwagonResponse, Create/UpdateSpeedwagonRequest
│   └── common.rs        # ErrorResponse 등 공통 타입
├── agent/
│   ├── mod.rs
│   └── spec.rs          # AgentSpec, AgentProvider (ailoy 타입의 API-safe 래퍼)
├── services/
│   ├── mod.rs
│   ├── session.rs       # 세션 CRUD, SSE 스트리밍, get_session_detail
│   ├── indexing.rs      # Speedwagon tantivy 인덱싱
│   └── speedwagon.rs    # Speedwagon 비즈니스 로직
└── repository/
    ├── mod.rs           # Repository 트레잇 + 팩토리
    ├── sqlite.rs        # SQLite 구현 (본 구현)
    └── postgres.rs      # PostgreSQL 구현 (스텁, 모든 메서드 todo!())
```

### Naming

| 대상 | 규칙 | 예시 |
|------|------|------|
| 파일/모듈 | snake_case | `handlers.rs`, `agent/spec.rs` |
| 함수/변수 | snake_case | `create_agent`, `provider_profile_id` |
| 구조체/Enum | PascalCase | `Agent`, `ProviderProfile`, `MessageRole` |
| 상수 | SCREAMING_SNAKE_CASE | `DEFAULT_DATABASE_URL` |

### Import 규칙

- `use` 문은 파일 최상단에 배치. **인라인 정규화 경로 금지** (`crate::models::X::new()` 등)
- 순서: std → 외부 크레이트 → `crate::` 내부 모듈
- 사용하지 않는 import는 즉시 제거
- 테스트 코드에서 1회성 사용은 인라인 경로 예외 가능

### API 규칙

| 항목 | 규칙 |
|------|------|
| URL | kebab-case, 리소스 중심 (`/provider-profiles/{id}`) |
| Method | REST 표준 (GET=조회, POST=생성, PUT=수정, DELETE=삭제) |
| 응답 코드 | 200(OK), 201(Created), 204(No Content), 400(Bad Request), 404(Not Found), 409(Conflict) |
| Body | JSON, `Content-Type: application/json` |
| 에러 | `ErrorResponse { error: String }` 통일 |
| 문서화 | utoipa `#[utoipa::path(...)]` 매크로로 OpenAPI 자동 생성 |

### 타입 패턴

모든 타입은 `models/` 폴더에서 도메인별로 관리한다. 파일 내 순서는 통일:

```
// --- Enums ---           열거형 (MessageRole, SourceType 등)
// --- Domain Models ---   DB 매핑용 내부 struct (Session, Agent 등)
// --- Response DTOs ---   API 응답 struct + From impl (SessionResponse 등)
// --- Request DTOs ---    API 입력 struct (CreateSessionRequest 등)
```

| 카테고리 | derive | 특징 |
|----------|--------|------|
| Domain Model | `Clone, Debug, Serialize, Deserialize` | DB 매핑, 내부 로직용 |
| Response DTO | `Clone, Debug, Serialize, ToSchema` | `XxxResponse` 접미사, `From<&Domain>` impl |
| Request DTO | `Debug, Deserialize, ToSchema` | `CreateXxxRequest` / `UpdateXxxRequest`, `deny_unknown_fields` |

용도에 따라 같은 리소스도 Response를 분리한다:
- `SessionResponse` — 목록/생성 (messages 없음)
- `SessionDetailResponse` — 상세 조회 (messages + tool_calls 포함)

### 튜플 From 패턴

두 독립적 데이터를 합쳐서 Response를 만들 때 `From<(A, B)> for C`를 구현한다:

```rust
// models/session.rs — 매핑 로직이 models에 캡슐화됨
impl From<(Session, Vec<SessionToolCall>)> for SessionDetailResponse {
    fn from((session, tool_calls): (Session, Vec<SessionToolCall>)) -> Self { ... }
}

// services/session.rs — service는 조회 + From 위임만
let session = repo.get_session(id).await?;
let tool_calls = repo.get_tool_calls_for_session(id).await?;
Ok(Some(SessionDetailResponse::from((session, tool_calls))))
```

도메인 모델에 불필요한 필드(예: `SessionMessage.tool_calls`)를 추가하지 않으면서도 변환 로직을 깔끔하게 캡슐화할 수 있다.

### 아키텍처 패턴

- **Repository 패턴**: `Repository` 트레잇 → `SqliteRepository` / `PostgresRepository` 구현
- **환경 변수 기반 설정**: `DATABASE_URL`, `BIND_ADDR`, `OPENAI_API_KEY` 등
- **요청 검증**: 핸들러 내에서 빈 문자열 체크, 비즈니스 로직 검증은 service에서
- **에러 처리 체인**: `RepositoryError` → `repository_error_response()` → `json_error()`
- **서비스 에러**: `SessionError` 등 `ResponseError` 구현 → `error.error_response()` 직접 사용

### 테스트

- 모듈 내 `#[cfg(test)] mod tests` 방식
- `#[actix_web::test]` 매크로
- 헬퍼 함수로 테스트 데이터 생성 (`create_agent()`, `create_provider_profile()`)
- `TempDir`로 임시 DB 사용
- SSE 엔드포인트 테스트: `parse_sse_events()`, `stream_user_message()` 헬퍼 사용

---

## chat-agent 크레이트

ailoy 런타임을 감싸는 독립 크레이트. Backend에서 `chat_agent::ChatAgent`로 사용한다.

### 구조

```
chat-agent/src/
├── lib.rs              # pub exports (ChatAgent, ChatEvent, ToolCallEntry, KbEntry 등)
├── chat_agent.rs       # ChatAgent 구조체 (런타임 생성, 스트리밍 실행, 히스토리 관리)
├── tools.rs            # 메인 에이전트 도구 (utc_now, add_integers, read_source)
└── speedwagon/
    ├── mod.rs           # KbEntry, SubAgentProvider, ASK_SPEEDWAGON_TOOL
    ├── dispatch.rs      # ask_speedwagon 도구 빌드 + 서브에이전트 실행
    └── indexing.rs      # tantivy 인덱스 빌드
```

### 도구 추가 패턴 (desc / func / build 3단 분리)

새 도구를 추가할 때 반드시 이 패턴을 따른다:

```rust
// 1. desc: 도구 설명 (ToolDesc)
fn my_tool_desc() -> ailoy::ToolDesc {
    ToolDescBuilder::new("my_tool")
        .description("도구 설명")
        .parameters(...)
        .build()
}

// 2. func: 실행 함수 (ToolFunc)
fn my_tool_func() -> ToolFunc {
    Arc::new(move |_name, args| Box::pin(async move { ... }))
}

// 3. build: desc + func를 조합하여 (name, ToolRuntime) 반환
pub fn build_my_tool(...) -> Option<(String, ToolRuntime)> {
    Some(("my_tool".to_string(), ToolRuntime::new(my_tool_desc(), my_tool_func())))
}
```

- `build_*` 함수가 `Option`을 반환하면 조건부 등록이 가능 (예: KB가 없으면 `ask_speedwagon` 미등록)
- 도구 이름은 상수로 정의 (`pub const MY_TOOL: &str = "my_tool"`)
- 간단한 도구는 `tools.rs`에, 복잡한 도구는 별도 모듈로 분리

### ChatEvent 스트리밍

`ChatAgent::run_user_text_streaming()`은 `Stream<Item = Result<ChatEvent, ChatAgentRunError>>`를 반환한다:

```
Thinking → [ToolCall → ToolResult]* → Message
```

- `Thinking`: LLM 호출 시작
- `ToolCall { tool, args }`: 도구 호출
- `ToolResult { tool, result }`: 도구 결과
- `Message { content, tool_calls }`: 최종 응답 + 전체 도구 호출 기록

### 히스토리 관리

- `get_history()` — 현재 런타임의 메시지 히스토리 반환
- `restore_history(messages)` — 외부(DB)에서 가져온 메시지를 런타임에 주입
- `trim_history()` — 20턴(user/assistant) 초과 시 가장 오래된 턴부터 제거 (시스템 메시지 보존)
- `update_system_prompt(prompt)` — 시스템 프롬프트 교체 (KB 변경 등)

### Speedwagon 서브에이전트

- `ask_speedwagon` 도구로 단발성 서브에이전트 생성 → RAG 검색 수행 → 결과 반환
- 서브에이전트는 부모 에이전트의 API credentials를 상속 (`SubAgentProvider`)
- KB별 모델 지정 가능 (`KbEntry.lm`), 없으면 부모 모델 사용
- 매 호출이 stateless — 이전 호출의 맥락을 기억하지 않음

---

## Frontend (TypeScript / Next.js)

### 프로젝트 구조

```
frontend/
├── app/                          # Next.js App Router
│   ├── layout.tsx               # 루트 레이아웃 (Server Component)
│   ├── page.tsx                 # / → /chat redirect
│   ├── chat/page.tsx
│   ├── sources/page.tsx
│   ├── settings/page.tsx
│   └── speedwagons/[id]/page.tsx
├── components/
│   ├── ui/                      # shadcn/ui 프리미티브 (수정 최소화)
│   ├── layout/                  # 레이아웃 (sidebar, theme)
│   ├── assistant-ui/            # assistant-ui Thread 래퍼
│   ├── chat/                    # 채팅 관련 (ApiRuntimeProvider, ChatView 등)
│   ├── sources/                 # Source 업로드/관리
│   ├── speedwagons/             # Speedwagon 상세/편집
│   └── settings/                # Provider 프로필 관리
├── lib/
│   ├── types.ts                 # 도메인 타입 정의 (중앙) — Response + Request 모두
│   ├── api.ts                   # Backend fetch 래퍼 + API 함수
│   ├── store.ts                 # Zustand 단일 스토어
│   ├── constants.ts             # Provider별 모델/설정 상수
│   └── utils.ts                 # cn() 유틸리티
```

### Naming

| 대상 | 규칙 | 예시 |
|------|------|------|
| 파일 | kebab-case | `api-runtime-provider.tsx`, `tool-call-block.tsx` |
| 컴포넌트 | PascalCase | `ChatView`, `SpeedwagonDetail`, `ModelSelector` |
| 함수/변수/훅 | camelCase | `useAppStore`, `handleFiles`, `isExpanded` |
| Props 인터페이스 | PascalCase + Props 접미사 | `ChatViewProps`, `SpeedwagonDetailProps` |
| 이벤트 핸들러 | on + 동사 | `onToggleKnowledgePanel`, `onDragOver` |

### 타입 관리 (`lib/types.ts`)

모든 도메인 타입은 `lib/types.ts`에서 중앙 관리한다. `api.ts`에 인라인 타입 정의 금지.

| 카테고리 | 네이밍 | 예시 |
|----------|--------|------|
| API 응답 | `ApiXxx` | `ApiSession`, `ApiAgent`, `ApiSessionMessage` |
| API 요청 | `CreateXxxRequest`, `UpdateXxxRequest` | `CreateSessionRequest`, `UpdateAgentRequest` |
| SSE 이벤트 | 그대로 | `SseEventType`, `SseEvent` |

`api.ts`는 `export type { ... }`로 re-export만 허용.

### 컴포넌트 규칙

| 항목 | 규칙 |
|------|------|
| 컴포넌트 | 함수형 컴포넌트 (`function Component()`) |
| "use client" | 인터랙티브 컴포넌트에만 명시. 레이아웃은 Server Component |
| Props | `interface XxxProps` 정의 후 구조분해 할당 |
| 내보내기 | named export (`export function`, `export const`) |

### assistant-ui API 스타일

- 0.12 신규 API 사용: `useAuiState((s) => s.thread.isRunning)` (O)
- legacy hooks 금지: `useThread`, `useThreadRuntime` (X, 0.13에서 제거 예정)
- 참고: https://assistant-ui.com/docs/migrations/v0-12

### 상태 관리 (Zustand)

- **단일 스토어**: `useAppStore` (`lib/store.ts`)
- **셀렉터 패턴**: `const activeSessionId = useAppStore((s) => s.activeSessionId)`
- **비-React 접근**: `useAppStore.getState().someAction()` (이벤트 핸들러, 어댑터 내부)
- **액션 네이밍**: 동사+명사 (`setActiveSession`, `updateSessionKnowledge`)
- **주의**: 셀렉터로 함수(액션)를 가져오면 리렌더 트리거 안 됨 → 데이터를 직접 구독해야 함

### 사이드바 세션 리스트 갱신 원칙

사이드바 `SessionList`는 `sessionListVersion`이 변경될 때만 re-fetch한다.
**Backend 상태가 변경되는 모든 시점**에서 `bumpSessionListVersion()`을 호출해야 한다.

필수 호출 시점:
- 세션 생성 직후 (새 세션이 목록에 나타나도록)
- 메시지 스트림 완료 후 (첫 메시지 기반 자동 title이 반영되도록)
- 세션 title 수동 변경 후
- 세션 삭제 후 (로컬 state로 이미 처리하지만, 일관성을 위해)

새로운 기능을 추가할 때 세션 목록에 영향을 주는 Backend 상태 변경이 있다면
반드시 `bumpSessionListVersion()` 호출을 포함해야 한다.

### 스타일링

| 항목 | 규칙 |
|------|------|
| 프레임워크 | Tailwind CSS 4 |
| 조건부 클래스 | `cn()` 유틸리티 (clsx + tailwind-merge) |
| 컴포넌트 변형 | CVA (class-variance-authority) |
| 색상 토큰 | 시맨틱 토큰 (`bg-background`, `text-foreground`, `bg-primary`) |
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
import type { ApiSession } from "@/lib/types";

// 4. 내부 유틸/스토어 (@/ alias)
import { cn } from "@/lib/utils";
import { useAppStore } from "@/lib/store";

// 5. 컴포넌트
import { Button } from "@/components/ui/button";
```

### TypeScript 규칙

- **interface**: Props, State, 도메인 타입 정의
- **type**: import type 문법으로 타입만 가져올 때
- **strict mode** 활성화
- **경로 alias**: `@/*` → 프로젝트 루트

---

## 공통 규칙

### Git

- **브랜치**: `main` (기본), `feat/xxx` 등 기능별 브랜치
- **커밋 메시지**: Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`)
- **Co-Author**: 자동 생성 코드에 Co-Authored-By 태그

### 코드 변경 시 주의

- 기존 네이밍 컨벤션을 변경하지 말 것 (예: snake_case → camelCase 전환 금지)
- 새 파일 추가 시 기존 디렉토리 구조를 따를 것
- API URL 패턴을 변경하지 말 것 (kebab-case 유지)
- shadcn/ui 컴포넌트(`components/ui/`)는 직접 수정 최소화
- **형태를 해치는 변경이 필요한 경우 반드시 사전 확인 필요**
