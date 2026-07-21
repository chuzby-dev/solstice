//! Runs the SOL/USDC paper-trading demo engine alongside the REST/
//! WebSocket API, so a dashboard (or `curl`/a WebSocket client) can watch
//! it trade in real time.
//!
//! ```sh
//! cargo run -p solstice-api --bin serve
//! ```
//!
//! REST: http://127.0.0.1:8080/api/v1/{status,positions,trades,performance}
//! WebSocket: ws://127.0.0.1:8080/api/v1/ws

use solstice_api::ApiServer;
use solstice_simulation::build_sol_usdc_demo_engine;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

fn main() {
    // See solstice-simulation's paper_trade.rs for why this needs a
    // larger-than-default stack on Windows debug builds.
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(run)
        .expect("failed to spawn main worker thread");
    handle.join().expect("main worker thread panicked");
}

fn run() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("failed to build tokio runtime")
        .block_on(async_main());
}

async fn async_main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let rpc_url = std::env::var("HELIUS_RPC_URL")
        .expect("HELIUS_RPC_URL not set (add it to .env at the repo root)");
    let addr: SocketAddr = std::env::var("SOLSTICE_API_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()
        .expect("SOLSTICE_API_ADDR must be a valid socket address, e.g. 127.0.0.1:8080");

    info!("Starting paper trading engine against live Solana mainnet data (read-only, no real transactions)");
    let engine = Arc::new(build_sol_usdc_demo_engine(rpc_url).await);

    let engine_for_loop = engine.clone();
    let trading_task = tokio::spawn(async move { engine_for_loop.run().await });

    let server = ApiServer::new(engine, addr);
    let server_task = tokio::spawn(async move {
        if let Err(e) = server.start().await {
            tracing::error!("API server error: {}", e);
        }
    });

    let _ = tokio::join!(trading_task, server_task);
}
