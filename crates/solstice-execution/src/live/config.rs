//! Configuration for [`super::engine::LiveTradingEngine`].

use crate::risk::RiskLimits;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
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
    /// Whether the strategy-driven signal pipeline (currently
    /// `SimpleMovingAverageStrategy`/`SpreadArbitrageStrategy`) runs at
    /// all. `true` by default -- this only exists so a caller can run
    /// *only* the cross-DEX arbitrage executor (`cross_dex_arb_enabled`)
    /// without also taking the directional bets those strategies place,
    /// without having to remove them from the engine's `StrategyManager`
    /// entirely. Independent of the main `enable`/`disable` kill switch:
    /// with this `false` and `cross_dex_arb_enabled: true`, an enabled
    /// engine trades only arbitrage opportunities. Adjustable at runtime
    /// via `LiveTradingEngine::set_strategies_enabled`.
    pub strategies_enabled: bool,
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
    /// Whether the dedicated cross-DEX arbitrage executor runs at all --
    /// buy on whichever registered DEX (Jupiter/Raydium/Orca) quotes
    /// cheapest for a pair, then immediately sell on whichever quotes
    /// priciest. Off by default: unlike every other trade this engine
    /// makes, this issues **two separate live transactions** back to
    /// back with real execution-price risk between them -- nothing in
    /// this workspace builds a single same-transaction atomic two-leg
    /// swap (that would require an on-chain program that reads the first
    /// leg's actual output rather than a pre-computed amount, which
    /// doesn't exist here). If the second leg fails after the first
    /// lands, the resulting inventory is tracked as a normal open
    /// position (protected by `stop_loss_percent`/`take_profit_percent`
    /// going forward) rather than lost track of -- but the arbitrage
    /// itself is not risk-free the way the name might suggest. Requires
    /// an explicit opt-in on top of `LiveTradingEngine::enable()`.
    /// Adjustable at runtime via
    /// `LiveTradingEngine::set_cross_dex_arb_enabled`.
    pub cross_dex_arb_enabled: bool,
    /// Minimum spread (e.g. `0.005` = 0.5%) between the cheapest and
    /// priciest quoted price for a pair, across every registered DEX,
    /// required to attempt a cross-DEX arbitrage trade. Set well above a
    /// single swap's round-trip cost: two separate swaps each pay their
    /// own fees and slippage tolerance (unlike a single-DEX trade), so
    /// this needs more headroom than `SpreadArbitrageStrategy`'s
    /// much-smaller `min_spread_bps` (which only ever bets directionally
    /// on one leg, not two). Paired with `cross_dex_max_slippage_bps`
    /// (default 0.3%/leg, ~0.6% round-trip) below -- a 0.5% threshold is
    /// only safe because that per-leg tolerance was tightened alongside
    /// it. Adjustable at runtime via
    /// `LiveTradingEngine::set_cross_dex_min_spread`.
    pub cross_dex_min_spread: f64,
    /// Slippage tolerance applied to each leg of a cross-DEX arbitrage
    /// trade, in basis points -- deliberately separate from
    /// `slippage_bps` below (which is much wider, tuned for ordinary
    /// directional trades where a missed fill just means a skipped
    /// opportunity). Here, a loose tolerance can silently erase the
    /// whole edge: if `cross_dex_min_spread` is set tight (say, 0.5%) but
    /// each of the two legs can slip against the trade by more than that,
    /// a "profitable" arb can net a real loss. Keep this well below
    /// `cross_dex_min_spread` divided by two. Adjustable at runtime via
    /// `LiveTradingEngine::set_cross_dex_max_slippage_bps`.
    pub cross_dex_max_slippage_bps: u32,
    /// Minimum profit margin, in basis points, required *after* assuming
    /// both legs slip against the trade by the full
    /// `cross_dex_max_slippage_bps` tolerance. `cross_dex_min_spread`
    /// alone only checks the raw quoted price gap -- it says nothing
    /// about whether that gap survives paying slippage on two separate,
    /// non-atomic transactions plus network/priority fees. The executor's
    /// real gate is `max(cross_dex_min_spread, 2 * cross_dex_max_slippage_bps
    /// + cross_dex_min_net_edge_bps)`, so raising the slippage tolerance
    /// automatically raises the spread required to trade instead of
    /// silently eating into the margin. Adjustable at runtime via
    /// `LiveTradingEngine::set_cross_dex_min_net_edge_bps`.
    pub cross_dex_min_net_edge_bps: u32,
    /// Labels (`LiveTradedPair::label`) excluded from *new* trade
    /// consideration -- both the cross-DEX arb's opportunity search and
    /// ordinary strategy market sampling skip a disabled pair, unless it
    /// already has an open position, in which case sampling/closing keep
    /// running for it regardless (stop-loss/take-profit and the arb's
    /// flatten-back-to-quote retry must never lose track of live
    /// inventory just because a pair was toggled off after the fact).
    /// Empty by default -- every configured pair trades unless explicitly
    /// disabled. Adjustable at runtime via
    /// `LiveTradingEngine::set_pair_enabled`.
    pub disabled_pairs: HashSet<String>,
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
            strategies_enabled: true,
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
            cross_dex_arb_enabled: false,
            cross_dex_min_spread: 0.005,
            cross_dex_max_slippage_bps: 30,
            cross_dex_min_net_edge_bps: 10,
            disabled_pairs: HashSet::new(),
            slippage_bps: 150,
            poll_interval: Duration::from_secs(15),
            tip_lamports: None,
        }
    }
}
