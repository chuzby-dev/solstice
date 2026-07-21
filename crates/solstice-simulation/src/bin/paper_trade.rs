//! Runnable paper-trading demo: polls live Solana mainnet prices (via the
//! RPC endpoint in `HELIUS_RPC_URL`) for SOL/USDC on Raydium and Orca,
//! runs the strategy framework against them, and logs simulated trades.
//! No real transactions are built or submitted — this only reads
//! on-chain state.
//!
//! ```sh
//! cargo run -p solstice-simulation --bin paper-trade
//! ```

use solstice_simulation::build_sol_usdc_demo_engine;
use tracing::info;

fn main() {
    // Run on a dedicated thread with a larger stack: Orca's tick-array
    // value types are several KB each and get moved through a few layers
    // of async calls, which overflows Windows' default 1MB thread stack
    // in unoptimized debug builds.
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

    info!("Starting paper trading engine against live Solana mainnet data (read-only, no real transactions)");
    let engine = build_sol_usdc_demo_engine(rpc_url).await;
    engine.run().await;
}
