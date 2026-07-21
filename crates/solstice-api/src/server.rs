//! Axum HTTP/WebSocket server wiring.

use crate::state::AppState;
use crate::{handlers, websocket};
use axum::routing::get;
use axum::Router;
use solstice_simulation::PaperTradingEngine;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

pub struct ApiServer {
    router: Router,
    addr: SocketAddr,
}

impl ApiServer {
    pub fn new(engine: Arc<PaperTradingEngine>, addr: SocketAddr) -> Self {
        let state = AppState::new(engine);

        let router = Router::new()
            .route("/api/v1/status", get(handlers::status))
            .route("/api/v1/positions", get(handlers::positions))
            .route("/api/v1/trades", get(handlers::trades))
            .route("/api/v1/performance", get(handlers::performance))
            .route("/api/v1/ws", get(websocket::ws_handler))
            .layer(CorsLayer::permissive())
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        ApiServer { router, addr }
    }

    /// The underlying Axum router, for embedding into a larger app or for
    /// tests that want to drive requests without binding a real socket
    /// (e.g. via `tower::ServiceExt::oneshot`). `Router` is cheaply
    /// cloneable (internally `Arc`-backed).
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// Bind and serve until the process is terminated.
    pub async fn start(self) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(self.addr).await?;
        info!("API server listening on http://{}", self.addr);
        axum::serve(listener, self.router).await
    }
}
