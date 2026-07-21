//! Integration tests: drive real HTTP requests (and a real WebSocket
//! connection) through `ApiServer`'s actual router against a real,
//! in-memory `PaperTradingEngine` — no live network calls, since the test
//! engine registers no Raydium/Orca pools, so `tick()` never reaches out
//! to a DEX.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use solana_sdk::pubkey::Pubkey;
use solstice_api::ApiServer;
use solstice_blockchain::SolanaRpcClient;
use solstice_dex::{OrcaClient, RaydiumClient};
use solstice_execution::risk::{
    ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits, RiskLimits,
};
use solstice_simulation::{MonitoredPair, PaperTradingConfig, PaperTradingEngine};
use solstice_strategy::{StrategyConfig, StrategyManager};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

fn test_risk_limits() -> RiskLimits {
    RiskLimits {
        position: PositionLimits {
            max_single_position_usd: 50_000,
            max_position_percent: 0.5,
            min_position_size_usd: 10,
            max_open_positions: 10,
        },
        daily_loss: DailyLossLimits {
            max_daily_loss_usd: 1_000_000,
            max_daily_loss_percent: 1.0,
        },
        exposure: ExposureLimits {
            max_total_exposure_usd: 1_000_000,
            max_leverage: 10.0,
        },
        concentration: ConcentrationLimits {
            max_single_asset_percent: 1.0,
        },
        order: OrderLimits {
            max_order_size_usd: 50_000,
            max_slippage_percent: 0.5,
        },
    }
}

/// Builds a real `PaperTradingEngine` with no Raydium/Orca pools
/// registered, so `sample_market`/`tick` never make a network call — this
/// lets tests exercise the real engine (not a mock) with no live RPC.
fn test_engine() -> Arc<PaperTradingEngine> {
    let rpc =
        Arc::new(SolanaRpcClient::with_endpoints(vec!["http://127.0.0.1:1".to_string()]).unwrap());
    let raydium = Arc::new(RaydiumClient::new(rpc.clone()));
    let orca = Arc::new(OrcaClient::new(rpc));
    let strategy_manager = Arc::new(StrategyManager::new(StrategyConfig::default()));

    let pair = MonitoredPair {
        pair: solstice_core::types::TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique()),
        label: "TEST/USDC",
        raydium_pool: None,
        orca_pool: None,
        reference_amount: 1_000_000_000,
    };

    let config = PaperTradingConfig {
        poll_interval: Duration::from_secs(3600),
        initial_capital_usd: 10_000.0,
        risk_limits: test_risk_limits(),
        kelly_fraction: 0.5,
        default_win_loss_ratio: 2.0,
        stop_loss_percent: 0.1,
    };

    Arc::new(PaperTradingEngine::new(
        raydium,
        orca,
        strategy_manager,
        vec![pair],
        config,
    ))
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn test_status_endpoint_reflects_engine_state() {
    let server = ApiServer::new(test_engine(), "127.0.0.1:0".parse().unwrap(), None);
    let response = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["status"], "running");
    assert_eq!(body["monitored_pairs"], serde_json::json!(["TEST/USDC"]));
    assert_eq!(body["open_positions"], 0);
    assert_eq!(body["total_value_usd"], 10_000.0);
    assert_eq!(body["circuit_breaker_tripped"], false);
}

#[tokio::test]
async fn test_positions_endpoint_empty_for_fresh_engine() {
    let server = ApiServer::new(test_engine(), "127.0.0.1:0".parse().unwrap(), None);
    let response = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/positions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["positions"], serde_json::json!([]));
}

#[tokio::test]
async fn test_trades_endpoint_empty_for_fresh_engine() {
    let server = ApiServer::new(test_engine(), "127.0.0.1:0".parse().unwrap(), None);
    let response = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/trades")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["trades"], serde_json::json!([]));
}

#[tokio::test]
async fn test_performance_endpoint_reflects_initial_capital() {
    let server = ApiServer::new(test_engine(), "127.0.0.1:0".parse().unwrap(), None);
    let response = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/performance")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["cash_usd"], 10_000.0);
    assert_eq!(body["realized_pnl_usd"], 0.0);
    assert_eq!(body["total_value_usd"], 10_000.0);
}

#[tokio::test]
async fn test_unknown_route_returns_404() {
    let server = ApiServer::new(test_engine(), "127.0.0.1:0".parse().unwrap(), None);
    let response = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_wallet_endpoint_404_when_not_configured() {
    let server = ApiServer::new(test_engine(), "127.0.0.1:0".parse().unwrap(), None);
    let response = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/wallet")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_wallet_endpoint_502_when_rpc_unreachable() {
    let wallet = solstice_api::WalletState {
        pubkey: Pubkey::new_unique(),
        rpc: Arc::new(
            SolanaRpcClient::with_endpoints(vec!["http://127.0.0.1:1".to_string()]).unwrap(),
        ),
    };
    let server = ApiServer::new(test_engine(), "127.0.0.1:0".parse().unwrap(), Some(wallet));
    let response = server
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/v1/wallet")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

/// Binds a real TCP listener and drives a real WebSocket handshake +
/// message against it — `tower::ServiceExt::oneshot` can't exercise a
/// protocol upgrade, since that hijacks the underlying connection.
#[tokio::test]
async fn test_websocket_streams_tick_completed_event() {
    let engine = test_engine();
    let server = ApiServer::new(engine.clone(), "127.0.0.1:0".parse().unwrap(), None);
    let router = server.router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let url = format!("ws://{addr}/api/v1/ws");
    let (mut ws_stream, _response) = tokio_tungstenite::connect_async(url).await.unwrap();

    // Give the server a moment to register the subscription before the
    // engine emits anything -- the broadcast channel only delivers to
    // subscribers already connected when the event fires.
    tokio::time::sleep(Duration::from_millis(50)).await;

    engine.tick().await.unwrap();

    let message = tokio::time::timeout(Duration::from_secs(5), ws_stream.next())
        .await
        .expect("timed out waiting for a WebSocket message")
        .expect("stream ended unexpectedly")
        .unwrap();

    let text = message.into_text().unwrap();
    let event: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(event["type"], "TickCompleted");

    ws_stream.close(None).await.ok();
}
