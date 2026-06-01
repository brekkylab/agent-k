use std::{collections::HashSet, sync::Arc};

use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{Mutex, broadcast, mpsc};
use uuid::Uuid;

use crate::{events::WsEvent, state::AppState};

#[derive(Deserialize)]
pub struct WsQueryParams {
    token: String,
}

/// Inbound control message from the client.
#[derive(Deserialize)]
struct ClientCommand {
    action: String,
    session_id: String,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsQueryParams>,
    State(state): State<Arc<AppState>>,
) -> Response {
    match state.jwt.decode(&params.token) {
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
        Ok(claims) => {
            let user_id = claims.sub;
            ws.on_upgrade(move |socket| handle_socket(socket, state, user_id))
        }
    }
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>, user_id: Uuid) {
    let (sink, mut stream) = socket.split();

    // Single writer: every task pushes Messages through this channel; the writer
    // task is the only owner of the SplitSink, serializing all outbound frames.
    let (tx, mut rx) = mpsc::channel::<Message>(256);

    // Sessions the client is actively spectating. Shared between the
    // broadcast-forward task (reads) and the inbound task (writes).
    let subscribed_sessions: Arc<Mutex<HashSet<Uuid>>> = Arc::new(Mutex::new(HashSet::new()));

    // ---- Task A: writer ----
    let mut writer_task = tokio::spawn(async move {
        let mut sink = sink;
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // ---- Task B: broadcast-forward ----
    let mut broadcast_rx = state.ws_tx.subscribe();
    let broadcast_tx = tx.clone();
    let broadcast_sessions = subscribed_sessions.clone();
    let mut broadcast_task = tokio::spawn(async move {
        loop {
            match broadcast_rx.recv().await {
                Ok(event) => {
                    // Determine whether this event should be forwarded and serialize it.
                    let forward = match &event {
                        // Forward title updates only to clients subscribed to that session.
                        WsEvent::SessionTitleUpdated { session_id, .. } => {
                            match Uuid::parse_str(session_id) {
                                Ok(sid) => broadcast_sessions.lock().await.contains(&sid),
                                Err(_) => false,
                            }
                        }
                        WsEvent::AgentRunStarted { session_id, .. }
                        | WsEvent::AgentMessage { session_id, .. }
                        | WsEvent::AgentError { session_id, .. }
                        | WsEvent::AgentRunDone { session_id } => match Uuid::parse_str(session_id)
                        {
                            Ok(sid) => broadcast_sessions.lock().await.contains(&sid),
                            Err(_) => false,
                        },
                        WsEvent::AgentRunIdle { session_id } => match Uuid::parse_str(session_id) {
                            Ok(sid) => broadcast_sessions.lock().await.contains(&sid),
                            Err(_) => false,
                        },
                    };

                    if !forward {
                        continue;
                    }

                    let Ok(json) = serde_json::to_string(&event) else {
                        continue;
                    };
                    // mpsc send failure means the writer is gone; silently stop.
                    if broadcast_tx.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "ws broadcast lagged; spectator may miss events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // ---- Task C: inbound ----
    let inbound_tx = tx.clone();
    let inbound_state = state.clone();
    let inbound_sessions = subscribed_sessions.clone();
    let mut inbound_task = tokio::spawn(async move {
        'inbound: while let Some(Ok(msg)) = stream.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => match String::from_utf8(b.to_vec()) {
                    Ok(s) => s,
                    Err(_) => continue,
                },
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => continue,
            };

            let Ok(cmd) = serde_json::from_str::<ClientCommand>(&text) else {
                continue;
            };

            let Ok(session_id) = Uuid::parse_str(&cmd.session_id) else {
                continue;
            };

            match cmd.action.as_str() {
                "subscribe" => {
                    // Authorize: any read access (Admin/ChatMember/ReadOnlyMember)
                    // is sufficient to spectate. Errors are silently ignored.
                    let authz = inbound_state
                        .repository
                        .get_session_with_authz(session_id, user_id)
                        .await;
                    let Ok(Some(_)) = authz else {
                        continue;
                    };

                    // Insert BEFORE snapshot so we don't miss live broadcasts between
                    // snapshot and replay completion. The client deduplicates by seq
                    // (Map.set is idempotent), so a duplicate AgentMessage is harmless.
                    inbound_sessions.lock().await.insert(session_id);

                    // Replay any in-progress run so the spectator catches up.
                    if let Some((user_message, outputs)) = inbound_state.snapshot(&session_id).await
                    {
                        let started = WsEvent::AgentRunStarted {
                            session_id: session_id.to_string(),
                            user_message,
                        };
                        if let Ok(json) = serde_json::to_string(&started) {
                            if inbound_tx.send(Message::Text(json.into())).await.is_err() {
                                break 'inbound;
                            }
                        }

                        for (seq, output) in outputs {
                            let event = WsEvent::AgentMessage {
                                session_id: session_id.to_string(),
                                seq,
                                output,
                            };
                            if let Ok(json) = serde_json::to_string(&event) {
                                if inbound_tx.send(Message::Text(json.into())).await.is_err() {
                                    break 'inbound;
                                }
                            }
                        }
                    } else {
                        // No active run — send AgentRunIdle so any client that is stuck
                        // in streaming=true (e.g. reconnect after run completion, or
                        // server restart mid-run) can reset its UI state.
                        let idle = crate::events::WsEvent::AgentRunIdle {
                            session_id: session_id.to_string(),
                        };
                        if let Ok(json) = serde_json::to_string(&idle) {
                            let _ = inbound_tx.send(Message::Text(json.into())).await;
                        }
                    }
                }
                "unsubscribe" => {
                    inbound_sessions.lock().await.remove(&session_id);
                }
                _ => continue,
            }
        }
    });

    // Drop the original tx now that all three tasks hold their own clones.
    // This allows the writer task to observe channel closure (rx.recv() → None)
    // and exit on its own when all senders are gone.
    drop(tx);

    // When any task finishes (socket closed, writer dead, broadcast closed),
    // abort the remaining two so the connection tears down cleanly.
    tokio::select! {
        _ = &mut writer_task => {}
        _ = &mut broadcast_task => {}
        _ = &mut inbound_task => {}
    }

    writer_task.abort();
    broadcast_task.abort();
    inbound_task.abort();
}
