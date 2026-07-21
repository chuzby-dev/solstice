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
    /// Known Raydium AMM v4 pool for this pair, if any -- pool addresses
    /// aren't derivable from a mint pair alone, so a pair with `None`
    /// here simply never wins the best-route comparison against Raydium,
    /// same as `MonitoredPair` in the paper-trading engine.
    pub raydium_pool: Option<Pubkey>,
    /// Known Orca Whirlpool for this pair, if any.
    pub orca_pool: Option<Pubkey>,
}

#[derive(Debug, Clone)]
pub struct LiveTradingConfig {
    /// Hard ceiling on total USD-equivalent capital this engine will ever
    /// deploy across all open positions, independent of the wallet's
    /// actual balance -- the wallet may hold more than this, and this cap
    /// is what actually limits risk, not the balance. Adjustable at
    /// runtime via `LiveTradingEngine::set_max_capital_usd`.
    pub max_capital_usd: f64,
    /// Minimum signal confidence (0.0-1.0) required to actually act on a
    /// signal -- anything below this is skipped (emits
    /// `LiveEvent::SignalSkipped`) rather than sized and traded, no matter
    /// how it would otherwise score. Adjustable at runtime via
    /// `LiveTradingEngine::set_min_confidence`.
    pub min_confidence: f64,
    pub risk_limits: RiskLimits,
    pub kelly_fraction: f64,
    pub default_win_loss_ratio: f64,
    pub stop_loss_percent: f64,
    /// Fractional gain (e.g. `0.05` = 5%) at which an open position is
    /// automatically closed. Without this, a position opened by a
    /// strategy that never itself signals an exit (SMA/SpreadArb, as
    /// currently configured) would only ever close on a loss (via
    /// `stop_loss_percent`) or sit indefinitely once capital is fully
    /// deployed. Adjustable at runtime via
    /// `LiveTradingEngine::set_take_profit_percent`.
    pub take_profit_percent: f64,
    /// Slippage tolerance passed to Jupiter for both price sampling and
    /// execution quotes. `execute_planned_trade` re-fetches a quote right
    /// before submitting, but `JupiterClient::build_swap_instructions`
    /// fetches its *own* fresh quote again internally (Jupiter's
    /// `/swap-instructions` needs the exact quote response body), so a
    /// live submission always trades against a slightly newer price than
    /// what the caller last saw. 50bps (0.5%) was tight enough that a
    /// live SOL/USDC attempt reverted on-chain with a Jupiter Route error
    /// (`0x1788`) most likely for exactly this reason -- widened to 150bps
    /// (1.5%) to give that price movement room without meaningfully
    /// affecting fill quality on the small trade sizes this engine targets.
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
            min_confidence: 0.65,
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
            take_profit_percent: 0.05,
            slippage_bps: 150,
            poll_interval: Duration::from_secs(15),
            tip_lamports: None,
        }
    }
}
