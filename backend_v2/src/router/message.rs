use std::{convert::Infallible, sync::Arc};

use ailoy::message::{Message, Part};
use axum::{
    Extension, Json,
    extract::{
        Path, Query, State,
        ws::{Message as WsMessage, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{
        Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::{
    auth::{AuthUser, authenticate},
    event::{MessageEvent, RunEvent, message_channel},
    state::AppState,
};

use super::{error::{ApiError, err}, workspace::require_owned_session};

/// A single persisted message together with its session-local sequence
/// number. Mirrors the `message/{id}` channel's [`MessageEvent`] shape so HTTP
/// catch-up and the WS stream are interchangeable on the client.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MessageResponse {
    pub seq: i64,
    pub message: Message,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MessageListResponse {
    pub items: Vec<MessageResponse>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PostMessageRequest {
    /// The user-turn content. Mirrors the `contents` of a `Role::User`
    /// [`ailoy::message::Message`].
    pub query: Vec<Part>,
}

#[derive(Debug, Deserialize)]
pub struct MessagesWsQuery {
    /// Bearer JWT — passed as a query parameter because browser `WebSocket`
    /// clients can't set custom headers on the upgrade request.
    pub token: String,

    /// Last seq the client already has. Resume forwards from `seq + 1`.
    /// Omit to receive from the start of the session.
    #[serde(default)]
    pub last_seq: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct StreamMessagesQuery {
    /// Last seq the client already has. Resume from `seq + 1`; omit for full history.
    #[serde(default)]
    pub last_seq: Option<i64>,
}

/// `GET /sessions/{id}/messages` — return the full persisted message history
/// for a session, ordered by `seq` ascending.
pub(super) async fn list_messages(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MessageListResponse>, ApiError> {
    require_owned_session(&state, &auth, id).await?;
    let messages = state.sessions.list_messages(id).await?;
    Ok(Json(MessageListResponse {
        items: messages
            .into_iter()
            .map(|(seq, message)| MessageResponse { seq, message })
            .collect(),
    }))
}

pub(super) async fn start_run(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
    Json(payload): Json<PostMessageRequest>,
) -> Result<StatusCode, ApiError> {
    require_owned_session(&state, &auth, id).await?;
    state.sessions.run(id, payload.query).await?;
    Ok(StatusCode::ACCEPTED)
}

pub(super) async fn stop_run(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_owned_session(&state, &auth, id).await?;
    if state.sessions.cancel(id).await {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(err(StatusCode::NOT_FOUND, "no active run"))
    }
}

/// Subscribe-then-catch-up: subscribe to the per-session channel first so any
/// publish concurrent with our DB catch-up is buffered into `rx`; then drain
/// rows with `seq > last_seq` from the DB; then forward live events filtered
/// by `seq > last_seq` (dedup against the catch-up). On `Lagged` we replay
/// the catch-up to reconcile.
pub(super) async fn stream_messages(
    State(state): State<Arc<AppState>>,
    Path(sid): Path<Uuid>,
    Query(query): Query<MessagesWsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    // Token via query (browser WebSockets can't set headers). authenticate +
    // require_owned_session apply the same gate as the HTTP routes.
    let user = authenticate(&state, &query.token).await?;
    require_owned_session(&state, &user, sid).await?;

    let last_seq = query.last_seq.unwrap_or(-1);

    Ok(ws.on_upgrade(move |mut socket| async move {
        let mut last_seq = last_seq;
        let channel = message_channel(sid);
        let mut rx = state.events.subscribe(&channel);

        loop {
            // Catch up from the DB. Entered once at start and again every
            // time the live-pump loop below breaks on `Lagged`.
            let rows = match state.sessions.list_messages_since(sid, last_seq).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(session = %sid, "ws catch-up DB error: {e}");
                    return;
                }
            };
            for (seq, message) in rows {
                let payload = match serde_json::to_string(&MessageEvent { seq, message }) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(session = %sid, "ws catch-up serialize error: {e}");
                        return;
                    }
                };
                if socket.send(WsMessage::Text(payload.into())).await.is_err() {
                    return;
                }
                last_seq = seq;
            }

            // Live pump until lag forces another catch-up or the channel closes.
            loop {
                match rx.recv().await {
                    Ok(payload) => {
                        let seq = serde_json::from_str::<serde_json::Value>(&payload)
                            .ok()
                            .and_then(|v| v.get("seq").and_then(|s| s.as_i64()));
                        let Some(seq) = seq else { continue };
                        if seq <= last_seq {
                            continue;
                        }
                        if socket.send(WsMessage::Text(payload.into())).await.is_err() {
                            return;
                        }
                        last_seq = seq;
                    }
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(session = %sid, missed, "ws subscriber lagged — reconciling from DB");
                        break;
                    }
                    Err(RecvError::Closed) => return,
                }
            }
        }
    }))
}

/// `GET /sessions/{id}/messages/stream` — SSE twin of `stream_messages` (WS).
/// Registered above `route_layer(auth_required)`, so the Bearer middleware wraps it
/// and authenticates via the normal `Authorization: Bearer` header; there is no `?token=` here.
/// Access is gated by [`require_owned_session`]: a session the caller can't
/// reach is reported as `404` (indistinguishable from missing).
///
/// Frames:
///   event: message   data: {"seq":N,"message":{...}}          (MessageEvent)
///   event: run       data: {"run":"started"|"finished"|"idle"} or
///                          {"run":"error","message":"..."}     (RunEvent)
///
/// A run-status snapshot (`started`/`idle`) is emitted after EVERY DB
/// catch-up (initial attach and post-Lagged reconciliation), so re-attaching
/// clients always learn whether a run is in flight.
pub(super) async fn stream_messages_sse(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(sid): Path<Uuid>,
    Query(query): Query<StreamMessagesQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    // Ownership + existence gate; maps both missing and non-owned to 404.
    require_owned_session(&state, &auth, sid).await?;
    let mut last_seq = query.last_seq.unwrap_or(-1);
    let channel = message_channel(sid);
    // Subscribe BEFORE catch-up so publishes concurrent with the DB read
    // buffer into `rx` instead of being lost. Also subscribe before the
    // `is_running` snapshot below: with the run-slot-removed-before-terminal-
    // publish invariant in `SessionsState::run`, a terminal event either
    // arrives on this open subscription or is already reflected in
    // `is_running == false` — never neither.
    let mut rx = state.events.subscribe(&channel);

    let stream = async_stream::stream! {
        loop {
            // Catch-up: entered at attach and again after every Lagged.
            let rows = match state.sessions.list_messages_since(sid, last_seq).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(session = %sid, "sse catch-up DB error: {e}");
                    return;
                }
            };
            for (seq, message) in rows {
                let data = match serde_json::to_string(&MessageEvent { seq, message }) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!(session = %sid, "sse catch-up serialize error: {e}");
                        return;
                    }
                };
                yield Ok(Event::default().event("message").data(data));
                last_seq = seq;
            }

            // Run-status snapshot so (re)attaching clients learn run state.
            let status = if state.sessions.is_running(sid).await {
                RunEvent::Started
            } else {
                RunEvent::Idle
            };
            match serde_json::to_string(&status) {
                Ok(data) => yield Ok(Event::default().event("run").data(data)),
                Err(e) => tracing::error!(session = %sid, "sse run-status serialize error: {e}"),
            }

            // Live pump until Lagged forces reconciliation or the channel
            // closes (session deleted → EventQueue::remove_channel).
            loop {
                match rx.recv().await {
                    Ok(payload) => {
                        let value = serde_json::from_str::<serde_json::Value>(&payload).ok();
                        let seq = value.as_ref().and_then(|v| v.get("seq")).and_then(|s| s.as_i64());
                        match seq {
                            Some(seq) if seq <= last_seq => continue, // dedup vs catch-up
                            Some(seq) => {
                                yield Ok(Event::default().event("message").data(payload));
                                last_seq = seq;
                            }
                            None => {
                                // Seq-less payloads are run lifecycle events.
                                if value.as_ref().is_some_and(|v| v.get("run").is_some()) {
                                    yield Ok(Event::default().event("run").data(payload));
                                }
                            }
                        }
                    }
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(session = %sid, missed, "sse subscriber lagged — reconciling from DB");
                        break;
                    }
                    Err(RecvError::Closed) => return,
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use tower::ServiceExt as _;
    use uuid::Uuid;

    use crate::{
        auth::{JwtConfig, Role, hash_password},
        router::get_router,
        state::{AppState, NewUser, Session, User},
    };

    /// Build an in-memory AppState with migrations applied.
    async fn make_state() -> Arc<AppState> {
        let jwt = JwtConfig::new("test-secret", 3600);
        let state = AppState::new("sqlite::memory:", PathBuf::from(std::env::temp_dir()), jwt)
            .await
            .expect("AppState::new failed");
        Arc::new(state)
    }

    /// Create a user (with its default workspace) and return the user plus a
    /// signed Bearer token for them.
    async fn create_user_and_token(state: &Arc<AppState>) -> (User, String) {
        let id = Uuid::new_v4();
        let password_hash = hash_password("Password1!").expect("hash");
        let user = state
            .users
            .create(NewUser {
                id,
                username: format!("testuser-{id}"),
                password_hash,
                role: Role::User,
                display_name: None,
                is_active: true,
                preferred_language: "en".to_string(),
            })
            .await
            .expect("create user");
        // Default workspace shares the user's id; sessions live inside it.
        state
            .workspaces
            .create_default(&user)
            .await
            .expect("create default workspace");
        let token = state
            .jwt
            .encode(id, format!("testuser-{id}"), Role::User)
            .expect("encode jwt");
        (user, token)
    }

    /// Insert a bare session (no sandbox) in the user's default workspace and
    /// return its id.
    async fn create_session(state: &Arc<AppState>, user: &User) -> Uuid {
        use agent_k::agents::get_coworker_agent_spec;

        // "openai" is an arbitrary model string; `with_skill = false` skips
        // any global tool registration that tests don't need.
        let spec = get_coworker_agent_spec("agent-k", "openai", false);
        let session = Session::new(user.id, spec);
        let sid = session.id;
        state
            .sessions
            .insert(session, None)
            .await
            .expect("insert session");
        sid
    }

    #[tokio::test]
    async fn sse_without_auth_returns_401() {
        let state = make_state().await;
        let app = get_router(state);
        let session_id = Uuid::new_v4(); // does not need to exist — 401 fires first
        let req = Request::builder()
            .method("GET")
            .uri(format!("/sessions/{session_id}/messages/stream"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn sse_for_existing_session_returns_200_text_event_stream() {
        let state = make_state().await;
        let (user, token) = create_user_and_token(&state).await;
        let sid = create_session(&state, &user).await;
        let app = get_router(state);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/sessions/{sid}/messages/stream"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.starts_with("text/event-stream"),
            "expected text/event-stream, got: {ct}"
        );
        // NOTE: the body is an infinite SSE stream — do NOT collect/await it.
        // Status + Content-Type are the contract we test here.
    }

    #[tokio::test]
    async fn sse_for_nonexistent_session_returns_404() {
        let state = make_state().await;
        let (_user, token) = create_user_and_token(&state).await;
        let phantom_id = Uuid::new_v4(); // never inserted
        let app = get_router(state);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/sessions/{phantom_id}/messages/stream"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
