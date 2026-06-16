use std::sync::Arc;

use aide::axum::{
    ApiRouter,
    routing::{get, post},
};
use ailoy::{agent::AgentSpec, message::Part};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        ws::{Message as WsMessage, WebSocketUpgrade},
    },
    http::StatusCode,
    response::Response,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::{
    auth::auth_required,
    event::{MessageEvent, message_channel},
    state::{AppState, Session, StateError},
};

use super::error::{ApiError, err};

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: Option<String>,
    pub spec: AgentSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Session> for SessionResponse {
    fn from(s: Session) -> Self {
        Self {
            id: s.id,
            project_id: s.project_id,
            title: s.title,
            spec: s.spec,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionListResponse {
    pub items: Vec<SessionResponse>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSessionRequest {
    pub project_id: Uuid,
    pub title: Option<String>,
    pub spec: AgentSpec,
}

pub fn get_session_router(state: Arc<AppState>) -> ApiRouter {
    // `route_layer` applies `auth_required` only to the HTTP routes added
    // above it. The WS route is appended afterward so it stays outside the
    // middleware — browser `WebSocket` clients can't send custom
    // `Authorization` headers, so [`stream_messages`] decodes a `?token=…`
    // query parameter inline instead.
    ApiRouter::new()
        .api_route("/sessions", get(list_sessions).post(create_session))
        .api_route("/sessions/{id}", get(get_session).delete(delete_session))
        .api_route("/sessions/{id}/messages", post(start_run))
        .api_route("/sessions/{id}/messages/stop", post(stop_run))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_required,
        ))
        .route(
            "/sessions/{id}/messages/ws",
            axum::routing::get(stream_messages),
        )
        .with_state(state)
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

async fn list_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let sessions = state.sessions.list().await?;
    Ok(Json(SessionListResponse {
        items: sessions.into_iter().map(SessionResponse::from).collect(),
    }))
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), ApiError> {
    if state.projects.get(payload.project_id).await?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "project not found"));
    }

    let mut session = Session::new(payload.project_id, payload.spec);
    if let Some(t) = payload.title {
        session = session.with_title(t);
    }
    state.sessions.insert(session.clone(), None).await?;
    Ok((StatusCode::CREATED, Json(SessionResponse::from(session))))
}

async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SessionResponse>, ApiError> {
    let session = state.sessions.get(id).await?.ok_or(StateError::NotFound)?;
    Ok(Json(SessionResponse::from(session)))
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.sessions.remove(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<PostMessageRequest>,
) -> Result<StatusCode, ApiError> {
    if state.sessions.get(id).await?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "session not found"));
    }
    state.sessions.run(id, payload.query).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn stop_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
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
async fn stream_messages(
    State(state): State<Arc<AppState>>,
    Path(sid): Path<Uuid>,
    Query(query): Query<MessagesWsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    // Mirrors `auth_required` on HTTP routes: JWT must decode. Token is
    // passed via query because browser WebSockets can't set headers.
    state.jwt.decode(&query.token)?;

    if state.sessions.get(sid).await?.is_none() {
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
