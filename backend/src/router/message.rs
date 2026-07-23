use std::sync::Arc;

use ailoy::message::{Message, Part};
use axum::{
    Extension, Json,
    extract::{
        Path, Query, State,
        ws::{Message as WsMessage, WebSocketUpgrade},
    },
    http::StatusCode,
    response::Response,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::{
    auth::{AuthUser, authenticate},
    event::{RunStatus, SessionEvent, message_channel},
    state::AppState,
};

use super::{
    error::{ApiError, err},
    workspace::require_owned_session,
};

/// A single persisted message together with its session-local sequence
/// number. Mirrors the `message/{id}` channel's [`SessionEvent::Message`]
/// shape (minus the `type` tag) so HTTP catch-up and the WS stream are
/// interchangeable on the client.
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
/// rows with `seq > last_seq` from the DB, followed by the in-flight turn's
/// partial text (as one cumulative delta) when a run is active; then forward
/// live events — `message` events filtered by `seq > last_seq` (dedup against
/// the catch-up), `delta`/`run` events verbatim (the client dedups deltas by
/// `cum_len`). On `Lagged` we replay the catch-up to reconcile.
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
                let payload = match serde_json::to_string(&SessionEvent::Message { seq, message })
                {
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

            // A run in flight when we attach: tell the client, then hand it
            // the in-progress turn's streamed-so-far text as one cumulative
            // delta so it doesn't wait for the next fragment. Live deltas
            // buffered in `rx` meanwhile overlap this snapshot; the client's
            // `cum_len` dedup drops them.
            if let Some(partial) = state.sessions.live_partial(sid).await {
                let mut events = vec![SessionEvent::Run {
                    status: RunStatus::Started,
                }];
                if !partial.is_empty() {
                    let cum_len = partial.encode_utf16().count() as u64;
                    events.push(SessionEvent::Delta {
                        text: partial,
                        cum_len,
                    });
                }
                for event in events {
                    let payload = match serde_json::to_string(&event) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!(session = %sid, "ws catch-up serialize error: {e}");
                            return;
                        }
                    };
                    if socket.send(WsMessage::Text(payload.into())).await.is_err() {
                        return;
                    }
                }
            }

            // Live pump until lag forces another catch-up or the channel closes.
            loop {
                match rx.recv().await {
                    Ok(payload) => {
                        // Only `message` events carry a seq and need dedup
                        // against the catch-up; everything else (`delta`,
                        // `run`) is ephemeral and forwarded verbatim.
                        let value: serde_json::Value = match serde_json::from_str(&payload) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        if value.get("type").and_then(|t| t.as_str()) == Some("message") {
                            let Some(seq) = value.get("seq").and_then(|s| s.as_i64()) else {
                                continue;
                            };
                            if seq <= last_seq {
                                continue;
                            }
                            if socket.send(WsMessage::Text(payload.into())).await.is_err() {
                                return;
                            }
                            last_seq = seq;
                        } else if socket.send(WsMessage::Text(payload.into())).await.is_err() {
                            return;
                        }
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
