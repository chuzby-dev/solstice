//! Shared setup for the SOL/USDC live-quote demo, used by both the
//! plain CLI binary and (via `solstice-api`) the API-serving binary, so
//! the pool addresses and risk configuration aren't duplicated.

use crate::{MonitoredPair, PaperTradingConfig, PaperTradingEngine};
use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::SolanaRpcClient;
use solstice_dex::{OrcaClient, RaydiumClient};
use solstice_execution::risk::{
    ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits, RiskLimits,
};
use solstice_strategy::strategies::{SimpleMovingAverageStrategy, SpreadArbitrageStrategy};
use solstice_strategy::{StrategyConfig, StrategyManager};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
// Verified live against Helius mainnet RPC before wiring in (see
// docs/CHANGELOG.md): owner/discriminator/mint fields checked, not
// guessed from memory.
const RAYDIUM_SOL_USDC_POOL: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";
const ORCA_SOL_USDC_WHIRLPOOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";

/// Build a `PaperTradingEngine` pre-configured to monitor SOL/USDC on
/// Raydium and Orca via the given RPC endpoint, with SMA and spread-arb
/// strategies registered and demo-appropriate (small, conservative) risk
/// limits.
pub async fn build_sol_usdc_demo_engine(rpc_url: String) -> PaperTradingEngine {
    let rpc = Arc::new(
        SolanaRpcClient::with_endpoints(vec![rpc_url]).expect("failed to build RPC client"),
    );

    let sol = Pubkey::from_str(SOL_MINT).expect("valid SOL mint");
    let usdc = Pubkey::from_str(USDC_MINT).expect("valid USDC mint");
    let raydium_pool = Pubkey::from_str(RAYDIUM_SOL_USDC_POOL).expect("valid Raydium pool");
    let orca_pool = Pubkey::from_str(ORCA_SOL_USDC_WHIRLPOOL).expect("valid Orca pool");

    let sol_usdc = solstice_core::types::TokenPair::new(sol, usdc);

    let raydium = Arc::new(RaydiumClient::new(rpc.clone()));
    let orca = Arc::new(OrcaClient::new(rpc.clone()));

    let strategy_manager = Arc::new(StrategyManager::new(StrategyConfig::default()));
    strategy_manager
        .register_strategy(Arc::new(SimpleMovingAverageStrategy::new(sol_usdc, 3, 10)))
        .await
        .expect("register SMA strategy");
    strategy_manager
        .register_strategy(Arc::new(SpreadArbitrageStrategy::new(10))) // 0.1% Raydium/Orca spread
        .await
        .expect("register spread-arb strategy");

    let risk_limits = RiskLimits {
        position: PositionLimits {
            max_single_position_usd: 1_000,
            max_position_percent: 0.2,
            min_position_size_usd: 10,
            max_open_positions: 5,
        },
        daily_loss: DailyLossLimits {
            max_daily_loss_usd: 500,
            max_daily_loss_percent: 0.1,
        },
        exposure: ExposureLimits {
            max_total_exposure_usd: 5_000,
            max_leverage: 1.0,
        },
        concentration: ConcentrationLimits {
            max_single_asset_percent: 0.5,
        },
        order: OrderLimits {
            max_order_size_usd: 1_000,
            max_slippage_percent: 0.02,
        },
    };

    let config = PaperTradingConfig {
        poll_interval: Duration::from_secs(15),
        initial_capital_usd: 10_000.0,
        risk_limits,
        kelly_fraction: 0.5,
        default_win_loss_ratio: 1.5,
        stop_loss_percent: 0.05,
    };

    let pairs = vec![MonitoredPair {
        pair: sol_usdc,
        label: "SOL/USDC",
        raydium_pool: Some(raydium_pool),
        orca_pool: Some(orca_pool),
        reference_amount: 1_000_000_000, // 1 SOL
    }];

    PaperTradingEngine::new(raydium, orca, strategy_manager, pairs, config)
}
