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
    event::{MessageEvent, message_channel},
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
    // Mirrors `auth_required` on HTTP routes via the shared `authenticate`
    // gate (token must decode to an active user). Token is passed via query
    // because browser WebSockets can't set headers. The session must live in
    // the caller's default workspace (whose id equals the user's id);
    // otherwise it's reported as 404 so it can't be probed.
    let user = authenticate(&state, &query.token).await?;

    let session = state
        .sessions
        .get(sid)
        .await?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "session not found"))?;
    // Same access gate as the HTTP routes: the session's workspace must be one
    // the caller can reach.
    if state
        .workspaces
        .get_for_user(user.id, session.workspace_id)
        .await?
        .is_none()
    {
        return Err(err(StatusCode::NOT_FOUND, "session not found"));
    }
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
