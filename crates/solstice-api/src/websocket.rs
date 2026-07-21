//! WebSocket endpoint: streams `EngineEvent`s to connected clients in
//! real time as newline-delimited JSON text frames.

use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tracing::{debug, warn};

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let mut rx = state.engine.subscribe();
    let (mut sender, mut receiver) = socket.split();

    // Drain (and ignore) inbound client messages so the socket stays
    // responsive to pings/close frames; this endpoint is publish-only.
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
    });

    loop {
        match rx.recv().await {
            Ok(event) => match serde_json::to_string(&event) {
                Ok(json) => {
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => warn!("Failed to serialize engine event: {}", e),
            },
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                debug!("WebSocket client lagged, skipped {} event(s)", skipped);
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }

    recv_task.abort();
}
