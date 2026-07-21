//! Configuration for [`super::engine::LiveTradingEngine`].

use crate::risk::RiskLimits;
use solana_sdk::pubkey::Pubkey;
use std::time::Duration;

/// A pair the live engine trades: the base token strategies signal on,
/// quoted against a stablecoin (or SOL) used to size and pay for trades.
#[derive(Debug, Clone, Copy)]
pub struct LiveTradedPair {
    pub label: &'static str,
    pub base_mint: Pubkey,
    pub base_decimals: u8,
    /// The token spent to buy `base_mint` and received when selling it
    /// (e.g. USDC). Assumed to be a 1-USD-equivalent stable for sizing
    /// purposes -- this engine does not price non-stable quote tokens.
    pub quote_mint: Pubkey,
    pub quote_decimals: u8,
    /// Amount of `base_mint` (in its raw base units) to request a quote
    /// for when sampling price, e.g. `10_000_000` for 0.01 SOL (9
    /// decimals) -- small enough to avoid meaningful price impact.
    pub reference_amount: u64,
}

#[derive(Debug, Clone)]
pub struct LiveTradingConfig {
    /// Hard ceiling on total USD-equivalent capital this engine will ever
    /// deploy across all open positions, independent of the wallet's
    /// actual balance -- the wallet may hold more than this, and this cap
    /// is what actually limits risk, not the balance. Adjustable at
    /// runtime via `LiveTradingEngine::set_max_capital_usd`.
    pub max_capital_usd: f64,
    pub risk_limits: RiskLimits,
    pub kelly_fraction: f64,
    pub default_win_loss_ratio: f64,
    pub stop_loss_percent: f64,
    pub slippage_bps: u32,
    pub poll_interval: Duration,
    /// Jito tip in lamports for each live submission. `None` skips the
    /// tip and relies on `submit_with_fallback`'s direct-RPC fallback.
    pub tip_lamports: Option<u64>,
}

impl Default for LiveTradingConfig {
    /// Deliberately conservative defaults -- a caller must explicitly
    /// raise `max_capital_usd` to trade with more than $50, not fall into
    /// it by omission.
    fn default() -> Self {
        LiveTradingConfig {
            max_capital_usd: 50.0,
            risk_limits: RiskLimits {
                position: crate::risk::PositionLimits {
                    max_single_position_usd: 50,
                    max_position_percent: 1.0,
                    min_position_size_usd: 1,
                    max_open_positions: 3,
                },
                daily_loss: crate::risk::DailyLossLimits {
                    max_daily_loss_usd: 50,
                    max_daily_loss_percent: 1.0,
                },
                exposure: crate::risk::ExposureLimits {
                    max_total_exposure_usd: 50,
                    max_leverage: 1.0,
                },
                concentration: crate::risk::ConcentrationLimits {
                    max_single_asset_percent: 1.0,
                },
                order: crate::risk::OrderLimits {
                    max_order_size_usd: 50,
                    max_slippage_percent: 0.05,
                },
            },
            kelly_fraction: 0.25,
            default_win_loss_ratio: 2.0,
            stop_loss_percent: 0.1,
            slippage_bps: 50,
            poll_interval: Duration::from_secs(15),
            tip_lamports: None,
        }
    }
}
