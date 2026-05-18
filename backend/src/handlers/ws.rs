use std::sync::Arc;

use axum::{
    extract::{Query, State, WebSocketUpgrade, ws::WebSocket},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{events::WsEvent, state::AppState};

#[derive(Deserialize)]
pub struct WsQueryParams {
    token: String,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsQueryParams>,
    State(state): State<Arc<AppState>>,
) -> Response {
    match state.jwt.decode(&params.token) {
        Err(_) => axum::http::StatusCode::UNAUTHORIZED.into_response(),
        Ok(_) => {
            let rx = state.ws_tx.subscribe();
            ws.on_upgrade(|socket| handle_socket(socket, rx))
        }
    }
}

async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<WsEvent>) {
    use axum::extract::ws::Message;
    loop {
        match rx.recv().await {
            Ok(event) => {
                let Ok(json) = serde_json::to_string(&event) else {
                    continue;
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
