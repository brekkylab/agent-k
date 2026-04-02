# API Design Issues

> 2026-03-31 기준 Backend API 설계 분석.
> 심각도순 정렬: High → Medium → Low

---

## 0. [High] Provider 삭제가 세션에 의해 차단됨 — UX 병목

**위치**: `handlers.rs:422-443`, `repository/mod.rs:62`

**현상**: `has_sessions_for_provider_profile()` 체크로 세션이 하나라도 참조하면 provider 삭제를 409로 차단. 세션이 누적되면 API 키 교체/provider 정리가 사실상 불가능. 옛 세션 때문에 새 키로 전환하지 못하는 상황 발생.

**영향**: 사용자가 API 키를 변경하거나 provider를 정리하려 할 때 모든 관련 세션을 먼저 삭제해야 함. 세션에 대화 기록이 있으면 기록 유실.

**수정 방향**: provider 삭제 시 연관 세션의 `provider_profile_id`를 NULL로 설정(cascade nullify). DB 스키마에서 `provider_profile_id`를 nullable로 변경. 세션 열 때 provider가 없으면 "모델을 다시 선택하세요" UI 표시. 기존 대화 히스토리는 보존.

---

## 1. [High] `AddSessionMessageRequest.role` — 과도한 입력 허용

**위치**: `models.rs:138-143`, `services/session.rs:131-151`, `frontend/lib/api.ts:212`

**현상**: 요청 DTO에 `role: MessageRole`(system/user/assistant/tool)을 모두 허용하지만, Frontend는 항상 `role: "user"` 고정. Backend 서비스에서 user가 아닌 role은 DB 저장만 하고 LLM 호출 없이 반환하는데, 이 경로는 실제로 사용되지 않음. 외부에서 임의 assistant 메시지를 대화 히스토리에 주입할 수 있는 취약점.

**수정 방향**: 요청 DTO에서 `role` 필드 제거, 서버가 항상 `MessageRole::User`로 고정.

---

## 2. [Medium] Source 삭제 비즈니스 로직이 핸들러에 직접 위치

**위치**: `handlers.rs:718-731`

**현상**: Source 삭제 시 연관 Speedwagon의 인덱스 상태를 리셋하는 로직이 핸들러 레이어에 직접 구현. `delete_speedwagon`이나 `create_speedwagon`은 `speedwagon_service`로 위임하는데 `delete_source`만 핸들러에 남아있음. 에러도 `let _ = ...`로 무시하며, 모든 Speedwagon을 풀스캔.

**수정 방향**: `services/source.rs`로 분리하고 에러 처리 정상화.

---

## 3. [Medium] `GET /sources/{id}` — Frontend 미사용

**위치**: `handlers.rs:678-686`

**현상**: Frontend `api.ts`에 `getSource(id)` 함수가 없음. `getSources()`(목록)와 `deleteSource(id)`만 사용. 엔드포인트 자체는 REST 완결성을 위해 유지할 수 있으나, 현재 데드 코드.

**수정 방향**: 당장 삭제 불필요. 향후 사용처 생기지 않으면 정리 대상.

---

## 4. [Medium] `updateSessionTitle` / `updateSession` Frontend 중복

**위치**: `frontend/lib/api.ts:177-199`

**현상**: 동일한 `PUT /sessions/{id}` 엔드포인트를 두 함수로 래핑.

```typescript
updateSessionTitle(id, title)   // { title }
updateSession(id, data)         // { title?, provider_profile_id?, ... }
```

**수정 방향**: `updateSessionTitle` 제거, `updateSession({ title })` 으로 통일.

---

## 5. [Medium] TOCTOU — delete_agent, delete_provider_profile

**위치**: `handlers.rs:280-296`, `427-443`

**현상**: `has_sessions_for_*` 체크 후 `delete_*` 호출이 별도 트랜잭션. 체크-삭제 사이에 새 세션이 생성되면 SQLite FK 위반이 `500 Internal Server Error`로 나감. `409 Conflict`가 적절.

**수정 방향**: FK 위반 에러를 `RepositoryError`에서 식별하여 409로 매핑. 또는 트랜잭션 내에서 체크+삭제 원자 실행.

---

## 6. [Low] `Session` 도메인 모델을 직접 응답으로 반환

**위치**: `handlers.rs:462`, `models.rs:62-72`

**현상**: Agent → `AgentResponse`, Source → `SourceResponse`, Speedwagon → `SpeedwagonResponse`로 변환하는데, `Session`만 도메인 모델을 그대로 반환. 현재는 문제없으나 내부 전용 필드 추가 시 직접 노출됨.

**수정 방향**: `SessionResponse` DTO 도입으로 패턴 통일.

---

## 7. [Low] `SpeedwagonResponse` — 서버 내부 경로 노출

**위치**: `models.rs:203-258`, `frontend/lib/types.ts:21-22`

**현상**: `index_dir`, `corpus_dir`(서버 파일시스템 절대 경로)가 클라이언트에 그대로 전달됨. Frontend에서 UI에 사용하지 않음.

**수정 방향**: `SpeedwagonResponse`에서 `index_dir`, `corpus_dir` 제외.

---

## 8. [Low] `assistant_message` nullable 과도

**위치**: `models.rs:145-148`

**현상**: `AddSessionMessageResponse.assistant_message`가 `Option<SessionMessage>`이지만, 정상 플로우(role=user)에서는 항상 값이 있음. null이 되는 경로는 issue #1에서 제거 대상인 non-user role 분기뿐.

**수정 방향**: issue #1 수정 후 `Option` 제거, `assistant_message: SessionMessage`로 변경 가능.

---

## 9. [Low] Speedwagon name/description 빈값 무검증

**위치**: `services/speedwagon.rs:50-68`

**현상**: `CreateProviderProfileRequest`는 `name.trim().is_empty()` 검사를 수행하지만, Speedwagon 생성/수정 시 name/description 빈값 검증 없음.

**수정 방향**: Speedwagon 서비스에 name 빈값 검증 추가.

---

## 10. [Low] `UpdateSessionRequest` — deny_unknown_fields 누락

**위치**: `models.rs:130-136`

**현상**: 다른 요청 DTO는 `#[serde(deny_unknown_fields)]`를 사용하는데 이 구조체만 누락.

**수정 방향**: `#[serde(deny_unknown_fields)]` 추가.

---

## 11. [Low] OpenAPI 문서 — 쿼리 파라미터 미문서화

**위치**: `handlers.rs:465-491`, `648-666`

**현상**: `list_sessions`의 `include_messages`, `limit`, `offset`과 `list_sources`/`list_speedwagons`의 `limit`, `offset`이 `#[utoipa::path]`의 `params()`에 문서화되어 있지 않음.

**수정 방향**: `params()` 매크로에 누락된 쿼리 파라미터 추가.

---

## 우선순위 로드맵

### Phase 1 — 보안/정합성
- [ ] **#1** — role 필드 제거 (보안 취약점)
- [ ] **#5** — TOCTOU + FK 위반 에러 매핑

### Phase 2 — 아키텍처 정리
- [ ] **#2** — Source 삭제 로직 서비스 레이어 분리
- [ ] **#4** — Frontend updateSessionTitle 중복 제거
- [ ] **#6** — SessionResponse DTO 도입

### Phase 3 — 응답 정리
- [ ] **#7** — index_dir/corpus_dir 노출 제거
- [ ] **#8** — assistant_message Optional 제거 (#1 선행)
- [ ] **#9** — Speedwagon name 검증
- [ ] **#10** — deny_unknown_fields 추가
- [ ] **#11** — OpenAPI params 보완

### 보류
- **#3** — GET /sources/{id} 사용처 생길 때까지 유지

### 향후 (SSE 도입 후)
- **Mutex 장시간 점유** — SSE 스트리밍 전체 시간 동안 세션별 TokioMutex Lock. 현재 단일 사용자이므로 수용 가능. 다중 사용자 세션 공유, 토큰 스트리밍 도입, SSE hung 연결 시 timeout 필요.
- **페이지네이션 (H)** — Frontend에 페이지네이션 UI가 필요해지는 시점에 구현
