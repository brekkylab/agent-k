# agentwebui_backend

Actix Web 기반 LLM Agent 백엔드입니다.  
현재 구현은 `ailoy` 런타임을 사용해 `POST /sessions/{id}/messages`에서 실제 모델 응답을 생성하며, 저장소는 SQLite를 기본으로 사용합니다.

## 프로젝트 개요

이 백엔드는 다음 컴포넌트를 제공합니다.

- `Agent`: 에이전트의 정체성/동작 설정(`AgentSpec`)
- `ProviderProfile`: 실행 시 사용할 LM Provider 설정(`AgentProvider`)
- `Session`: Agent + ProviderProfile 조합으로 생성되는 대화 단위
- `SessionMessage`: Session에 누적되는 메시지 로그

핵심 목적은 다음과 같습니다.

- Agent/ProviderProfile/Session을 API로 관리
- 세션 메시지 입력 시 모델 inference 수행
- 메시지/세션 상태를 RDB(SQLite)에 영속 저장

## 현재 상태 요약

- 운영 기본 DB: SQLite (`sqlite://./data/app.db`)
- Postgres 경로: 코드에 명시되어 있으나 `todo!("postgres implementation")`로 fail-fast
- Swagger UI: 내장 (`/swagger-ui`)
- OpenAPI JSON: `/api-docs/openapi.json`

## 실행 및 개발 명령

### 0) 바로 실행
```
OPENAI_API_KEY=sk-proj-... BIND_ADDR=0.0.0.0:8080 cargo run
```

### 1) 로컬 실행

```bash
cargo run
```

기본 바인드 주소는 `127.0.0.1:8080`입니다.

### 2) 테스트 실행

```bash
cargo test
```

### 3) 상태 확인

```bash
curl -sS http://127.0.0.1:8080/health
```

### 4) API 문서 확인

- Swagger UI: `http://127.0.0.1:8080/swagger-ui`
- OpenAPI JSON: `http://127.0.0.1:8080/api-docs/openapi.json`

## 환경변수

`.env.template`을 복사해 `.env`를 만들고 값을 채워 사용할 수 있습니다.

```bash
cp .env.template .env
```

| 변수 | 기본값 | 설명 |
|---|---|---|
| `BIND_ADDR` | `127.0.0.1:8080` | 서버 bind 주소 |
| `DATABASE_URL` | `sqlite://./data/app.db` | DB 연결 URL |
| `OPENAI_API_KEY` | 없음 | 서버 시작 시 `openai-default` profile bootstrap |
| `ANTHROPIC_API_KEY` | 없음 | 서버 시작 시 `anthropic-default` profile bootstrap |
| `GEMINI_API_KEY` | 없음 | 서버 시작 시 `gemini-default` profile bootstrap |

### ProviderProfile bootstrap 동작

서버 시작 시 위 API 키가 존재하면 해당 이름의 profile을 upsert(덮어쓰기)합니다.

- `openai-default`
- `anthropic-default`
- `gemini-default`

각 profile은 `is_default=true`로 유지됩니다.

## 데이터/아키텍처

### Repository 추상화

저장소 계층은 `Repository trait`로 추상화되어 있습니다.

- `SqliteRepository`: 실제 동작 구현
- `PostgresRepository`: 시그니처만 있고 본문은 `todo!()`

`DATABASE_URL` 스킴으로 구현체를 선택합니다.

- `sqlite://` 또는 미지정: SQLite 사용
- `postgres://`, `postgresql://`: Postgres 경로 진입 후 fail-fast (`todo!`)

### 무결성 제약

- `sessions.agent_id -> agents.id`: `ON DELETE RESTRICT`
- `sessions.provider_profile_id -> provider_profiles.id`: `ON DELETE RESTRICT`
- `session_messages.session_id -> sessions.id`: `ON DELETE CASCADE`

API 레벨에서도 아래 삭제 제한을 유지합니다.

- Session이 존재하는 Agent 삭제 시 `409`
- Session이 존재하는 ProviderProfile 삭제 시 `409`

## 런타임 동작 상세 (`POST /sessions/{id}/messages`)

요청 바디:

```json
{
  "role": "user",
  "content": "..."
}
```

응답 바디:

```json
{
  "assistant_message": {
    "role": "assistant",
    "content": "...",
    "created_at": "..."
  }
}
```

처리 흐름은 다음과 같습니다.

1. 입력 메시지를 DB에 먼저 저장
2. `role != user`면 inference 없이 종료 (`assistant_message: null`)
3. `role == user`면 세션 runtime으로 `runtime.run(...)` 실행
4. 모델 응답에서 텍스트 파트만 순서대로 이어붙여 assistant 메시지 생성
5. assistant 메시지를 DB에 저장 후 반환

실패 정책:

- provider/runtime 호출 실패: `502`
- 모델 응답에 텍스트 파트가 없으면: `502`
- 위 실패 상황에서도 user 메시지는 이미 저장된 상태를 유지

## Runtime Cache 정책 (중요)

세션별 in-memory runtime cache를 사용합니다.

- 키: `session_id`
- 값: `AgentRuntime` + 캐시 생성 시점의 `agent_id`, `provider_profile_id`

정책:

- 동일 세션의 연속 user 턴은 같은 runtime을 재사용(문맥 유지)
- `PUT /agents/{id}` 성공 시 해당 `agent_id` 관련 세션 runtime 전부 제거
- `PUT /provider-profiles/{id}` 성공 시 해당 `provider_profile_id` 관련 세션 runtime 전부 제거
- `DELETE /sessions/{id}` 성공 시 해당 세션 runtime 제거
- 서버 재시작 시 runtime cache는 복원되지 않음

주의:

- 서버 재시작 후에는 DB에 메시지 기록이 남아 있어도 runtime in-memory history는 초기화됩니다.

## API 사용 예시 (curl)

아래 예시는 `jq`가 설치되어 있다고 가정합니다.

### 1) Agent 생성

```bash
AGENT_ID=$(curl -sS -X POST http://127.0.0.1:8080/agents \
  -H 'content-type: application/json' \
  -d '{
    "spec": {
      "lm": "gpt-4.1-mini",
      "instruction": "You are a concise assistant.",
      "tools": []
    }
  }' | jq -r '.id')
```

### 2) ProviderProfile 선택 (`openai-default`)

```bash
PROFILE_ID=$(curl -sS http://127.0.0.1:8080/provider-profiles \
  | jq -r '.[] | select(.name=="openai-default") | .id' | head -n1)
```

### 3) Session 생성

```bash
SESSION_ID=$(curl -sS -X POST http://127.0.0.1:8080/sessions \
  -H 'content-type: application/json' \
  -d "{\"agent_id\":\"${AGENT_ID}\",\"provider_profile_id\":\"${PROFILE_ID}\"}" \
  | jq -r '.id')
```

### 4) 메시지 전송 (inference)

```bash
curl -sS -X POST "http://127.0.0.1:8080/sessions/${SESSION_ID}/messages" \
  -H 'content-type: application/json' \
  -d '{"role":"user","content":"Give me a one-line greeting."}'
```

### 5) 세션 메시지 확인

```bash
curl -sS "http://127.0.0.1:8080/sessions/${SESSION_ID}"
```

## 운영 트러블슈팅

### 400 Bad Request

- 요청 body 스키마 불일치 (`deny_unknown_fields`)
- 메시지 `content`가 빈 문자열
- 기본 provider profile이 없는데 session 생성 시 profile 미지정

### 404 Not Found

- 존재하지 않는 Agent / ProviderProfile / Session ID
- 세션 생성 시 명시한 `provider_profile_id`가 없음

### 409 Conflict

- Session이 남아있는 Agent를 삭제하려는 경우
- Session이 남아있는 ProviderProfile을 삭제하려는 경우

### 502 Bad Gateway

- `POST /sessions/{id}/messages`에서 LM provider 호출 실패
- API 키 누락/오류, endpoint 접근 실패, 업스트림 오류
- 모델 응답에 텍스트 파트가 없어 assistant 본문을 만들 수 없음

### API 키 관련 점검

- bootstrap 기대 시: 서버 프로세스에 `OPENAI_API_KEY` 등 환경변수가 실제로 주입됐는지 확인
- profile 생성/수정 API 사용 시: 요청에는 `api_key`를 보낼 수 있으나, 조회 응답에서는 마스킹(`null`)됨

## 참고

- Swagger 스키마는 API DTO 기준으로 노출됩니다.
- 내부 런타임/저장소는 ailoy 타입을 사용하며, API 경계에서 변환합니다.
