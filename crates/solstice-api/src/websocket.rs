//! WebSocket endpoints: stream `EngineEvent`s (paper) or `LiveEvent`s
//! (live) to connected clients in real time as JSON text frames.

use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::{debug, warn};

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.engine.subscribe();
    ws.on_upgrade(move |socket| stream_events(socket, rx))
}

pub async fn live_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> axum::response::Response {
    match &state.live {
        Some(live) => {
            let rx = live.subscribe();
            ws.on_upgrade(move |socket| stream_events(socket, rx))
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "no live trading engine configured").into_response(),
    }
}

/// Drains (and ignores) inbound client messages so the socket stays
/// responsive to pings/close frames; both endpoints are publish-only.
async fn stream_events<T: Serialize + Clone + Send + 'static>(
    socket: WebSocket,
    mut rx: broadcast::Receiver<T>,
) {
    let (mut sender, mut receiver) = socket.split();

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
                Err(e) => warn!("Failed to serialize event: {}", e),
            },
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                debug!("WebSocket client lagged, skipped {} event(s)", skipped);
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }

    recv_task.abort();
}
