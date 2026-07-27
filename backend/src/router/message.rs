use std::{convert::Infallible, sync::Arc};

use ailoy::message::{Message, Part};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    event::{RunStatus, SessionEvent, message_channel},
    state::AppState,
};

use super::{
    error::{ApiError, err},
    workspace::require_owned_session,
};

/// A single persisted message together with its session-local sequence
/// number. Mirrors the `message/{id}` channel's [`SessionEvent::Message`]
/// shape (minus the `type` tag) so HTTP catch-up and the SSE stream are
/// interchangeable on the client.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MessageResponse {
    pub seq: i64,
    /// Nesting level: `0` is the top-level conversation, `>= 1` a sub-agent's
    /// internal output.
    pub depth: u8,
    /// Name of the agent that produced the message, when known.
    pub source_agent: Option<String>,
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
pub struct StreamQuery {
    /// Last seq the client already has. Resume forwards from `seq + 1`.
    /// Omit to receive from the start of the session. On an SSE auto-reconnect
    /// the `Last-Event-ID` header takes precedence over this.
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
            .map(|(seq, stored)| MessageResponse {
                seq,
                depth: stored.depth,
                source_agent: stored.source_agent,
                message: stored.message,
            })
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

/// `GET /sessions/{id}/messages/stream` — the session's live event stream as
/// SSE, carrying [`SessionEvent`] JSON in each event's `data`.
///
/// Subscribe-then-catch-up: subscribe to the per-session channel first so any
/// publish concurrent with our DB catch-up is buffered into `rx`; then drain
/// rows with `seq > last_seq` from the DB, followed by the in-flight turn's
/// partial text (as one cumulative delta) when a run is active; then forward
/// live events — `message` events filtered by `seq > last_seq` (dedup against
/// the catch-up), `delta`/`run` events verbatim (the client dedups deltas by
/// `cum_len`). On `Lagged` we replay the catch-up to reconcile.
///
/// Only `message` events carry an SSE `id` (their `seq`), so the browser's
/// `Last-Event-ID` on auto-reconnect resumes from the persisted cursor —
/// ephemeral `delta`/`run` events never advance it.
pub(super) async fn stream_messages(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(sid): Path<Uuid>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    require_owned_session(&state, &auth, sid).await?;

    let last_seq = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .or(query.last_seq)
        .unwrap_or(-1);

    let stream = async_stream::stream! {
        let mut last_seq = last_seq;
        let channel = message_channel(sid);
        let mut rx = state.events.subscribe(&channel);

        loop {
            // Catch up from the DB. Entered once at start and again every
            // time the live-pump loop below breaks on `Lagged`.
            let rows = match state.sessions.list_messages_since(sid, last_seq).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(session = %sid, "sse catch-up DB error: {e}");
                    return;
                }
            };
            for (seq, stored) in rows {
                let event = SessionEvent::Message {
                    seq,
                    depth: stored.depth,
                    source_agent: stored.source_agent,
                    message: stored.message,
                };
                let payload = match serde_json::to_string(&event) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(session = %sid, "sse catch-up serialize error: {e}");
                        return;
                    }
                };
                yield Ok(Event::default().id(seq.to_string()).data(payload));
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
                            tracing::error!(session = %sid, "sse catch-up serialize error: {e}");
                            return;
                        }
                    };
                    yield Ok(Event::default().data(payload));
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
                            yield Ok(Event::default().id(seq.to_string()).data(payload));
                            last_seq = seq;
                        } else {
                            yield Ok(Event::default().data(payload));
                        }
                    }
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(session = %sid, missed, "sse subscriber lagged — reconciling from DB");
                        break;
                    }
                    // Channel removed (session deleted) — end the stream; a
                    // reconnect attempt will get a 404 from the ownership gate.
                    Err(RecvError::Closed) => return,
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
