//! Runs the SOL/USDC paper-trading demo engine (and, if a wallet is
//! configured, an automated live-trading engine -- disabled by default)
//! alongside the REST/WebSocket API.
//!
//! ```sh
//! cargo run -p solstice-api --bin serve
//! ```
//!
//! REST: http://127.0.0.1:8080/api/v1/{status,positions,trades,performance,wallet}
//! Live control: http://127.0.0.1:8080/api/v1/live/{status,enable,disable,config}
//! WebSocket: ws://127.0.0.1:8080/api/v1/ws (paper), /api/v1/live/ws (live)

use solana_sdk::pubkey::Pubkey;
use solstice_api::{ApiServer, ConvertState, WalletState};
use solstice_blockchain::{SolanaRpcClient, WalletFile};
use solstice_dex::JupiterClient;
use solstice_execution::jito::{JitoClient, JitoConfig};
use solstice_execution::{LiveTradedPair, LiveTradingConfig, LiveTradingEngine};
use solstice_simulation::build_sol_usdc_demo_engine;
use solstice_strategy::strategies::{SimpleMovingAverageStrategy, SpreadArbitrageStrategy};
use solstice_strategy::{StrategyConfig, StrategyManager};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tracing::{info, warn};

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
// Same addresses used (and independently verified against Raydium's and
// Orca's own public APIs) in `solstice_simulation::demo`'s paper-trading
// setup -- see that module for how these were confirmed.
const RAYDIUM_SOL_USDC_POOL: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";
const ORCA_SOL_USDC_WHIRLPOOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";

// RAY/USDC: much thinner liquidity than SOL/USDC (Raydium pool ~$3.9M
// TVL vs SOL/USDC's hundreds of millions; Orca pool ~$10K TVL), so it's
// less picked-over by fast professional arb bots and has shown a real,
// persistent cross-DEX spread (~0.5-0.6% observed directly via Raydium's
// and Orca's own public APIs at the time these addresses were verified)
// -- unlike SOL/USDC, where that spread is usually arbed away in
// sub-second time. Addresses independently confirmed against each
// protocol's own API (Raydium's `/pools/info/mint?poolType=standard`,
// Orca's `/v2/solana/pools?tokensBothOf=...`), the same way the SOL/USDC
// pair above was.
const RAY_MINT: &str = "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R";
const RAYDIUM_RAY_USDC_POOL: &str = "6UmmUiYoBjSrhakAobJw8BvkmJtDVxaeBtbt7rxWo1mg";
const ORCA_RAY_USDC_WHIRLPOOL: &str = "A2J7vmG9xAdWUzYscN7oQssxZBFihwD3UonkWB8Kod1A";

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
    let engine = Arc::new(build_sol_usdc_demo_engine(rpc_url.clone()).await);

    let engine_for_loop = engine.clone();
    let trading_task = tokio::spawn(async move { engine_for_loop.run().await });

    // Optional: if a wallet keypair file is configured, expose its public
    // address and balance read-only via /api/v1/wallet, and additionally
    // stand up a live-trading engine (kill switch defaults to OFF --
    // configuring a wallet alone never causes real trades). No private
    // key material is loaded at startup -- only the public key; the live
    // engine loads the signing key from disk itself, transiently, only at
    // the moment it needs to sign a real transaction.
    let mut wallet = None;
    let mut live = None;
    let mut convert = None;

    if let Ok(path) = std::env::var("WALLET_KEYPAIR_PATH") {
        let wallet_file = WalletFile::at(&path);
        if !wallet_file.exists() {
            warn!(
                "WALLET_KEYPAIR_PATH is set to {path} but no wallet file exists there yet; \
                 /api/v1/wallet and /api/v1/live/* will return 404 until one is generated"
            );
        } else {
            match wallet_file.pubkey() {
                Ok(pubkey) => {
                    info!("Wallet configured: {pubkey}");
                    let rpc = Arc::new(
                        SolanaRpcClient::with_endpoints(vec![rpc_url.clone()])
                            .expect("failed to build wallet RPC client"),
                    );
                    wallet = Some(WalletState {
                        pubkey,
                        rpc: rpc.clone(),
                    });

                    let sol = Pubkey::from_str(SOL_MINT).expect("SOL_MINT is a valid pubkey");
                    let usdc = Pubkey::from_str(USDC_MINT).expect("USDC_MINT is a valid pubkey");

                    match JitoClient::new(JitoConfig::default()) {
                        Ok(jito) => match JupiterClient::new() {
                            Ok(dex) => {
                                convert = Some(Arc::new(ConvertState {
                                    wallet_file: WalletFile::at(&path),
                                    wallet_pubkey: pubkey,
                                    rpc: rpc.clone(),
                                    jito,
                                    dex,
                                    sol_mint: sol,
                                    sol_decimals: 9,
                                    usdc_mint: usdc,
                                    usdc_decimals: 6,
                                }));
                            }
                            Err(e) => {
                                warn!("Failed to configure wallet conversion (Jupiter client): {e}")
                            }
                        },
                        Err(e) => {
                            warn!("Failed to configure wallet conversion (Jito client): {e}")
                        }
                    }
                    let pair = LiveTradedPair {
                        label: "SOL/USDC",
                        base_mint: sol,
                        base_decimals: 9,
                        quote_mint: usdc,
                        quote_decimals: 6,
                        reference_amount: 10_000_000, // 0.01 SOL
                        raydium_pool: Pubkey::from_str(RAYDIUM_SOL_USDC_POOL).ok(),
                        orca_pool: Pubkey::from_str(ORCA_SOL_USDC_WHIRLPOOL).ok(),
                    };

                    let ray = Pubkey::from_str(RAY_MINT).expect("RAY_MINT is a valid pubkey");
                    let ray_usdc_pair = LiveTradedPair {
                        label: "RAY/USDC",
                        base_mint: ray,
                        base_decimals: 6,
                        quote_mint: usdc,
                        quote_decimals: 6,
                        reference_amount: 1_000_000, // 1 RAY
                        raydium_pool: Pubkey::from_str(RAYDIUM_RAY_USDC_POOL).ok(),
                        orca_pool: Pubkey::from_str(ORCA_RAY_USDC_WHIRLPOOL).ok(),
                    };

                    let live_strategies = Arc::new(StrategyManager::new(StrategyConfig::default()));
                    live_strategies
                        .register_strategy(Arc::new(SimpleMovingAverageStrategy::new(
                            solstice_core::types::TokenPair::new(sol, usdc),
                            5,
                            20,
                        )))
                        .await
                        .expect("failed to register live SMA strategy");
                    live_strategies
                        .register_strategy(Arc::new(SpreadArbitrageStrategy::new(10))) // 0.1% Raydium/Orca spread
                        .await
                        .expect("failed to register live SpreadArb strategy");

                    // Reload the wallet file separately: `LiveTradingEngine`
                    // owns its own `WalletFile` handle (it re-reads the
                    // keypair from disk only at the moment it signs).
                    match LiveTradingEngine::new(
                        WalletFile::at(&path),
                        rpc,
                        live_strategies,
                        vec![pair, ray_usdc_pair],
                        LiveTradingConfig::default(),
                    ) {
                        Ok(engine) => {
                            info!(
                                "Live trading engine configured (DISABLED by default, max ${:.2} capital cap) -- \
                                 enable it via POST /api/v1/live/enable",
                                LiveTradingConfig::default().max_capital_usd
                            );
                            live = Some(Arc::new(engine));
                        }
                        Err(e) => warn!("Failed to configure live trading engine: {e}"),
                    }
                }
                Err(e) => {
                    warn!("Failed to load wallet at {path}: {e}");
                }
            }
        }
    }

    let live_task = live
        .clone()
        .map(|live: Arc<LiveTradingEngine>| tokio::spawn(async move { live.run().await }));

    let server = ApiServer::new(engine, addr, wallet, live, convert);
    let server_task = tokio::spawn(async move {
        if let Err(e) = server.start().await {
            tracing::error!("API server error: {}", e);
        }
    });

    if let Some(live_task) = live_task {
        let _ = tokio::join!(trading_task, server_task, live_task);
    } else {
        let _ = tokio::join!(trading_task, server_task);
    }
}
