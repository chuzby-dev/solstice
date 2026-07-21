//! Simulated order execution: slippage, fees, and partial fills.
//!
//! [`crate::engine::PaperTradingEngine`] fills every order instantly at the
//! live quote's exact price with zero fee, because a real DEX quote
//! already prices in the venue's actual liquidity and fee schedule. A
//! historical CSV tick carries none of that — it's a bare price point — so
//! a backtest needs an explicit, honest model for what a fill against that
//! price would have really cost, rather than reusing the live engine's
//! free/instant/unlimited-size fill and silently overstating every
//! strategy's backtested performance.

use chrono::{DateTime, Utc};
use solstice_execution::Fill;

/// Models the execution-price cost of trading against a bare price point.
#[derive(Debug, Clone, Copy)]
pub enum SlippageModel {
    /// Fills happen exactly at the tick price. Useful for isolating a
    /// strategy's raw signal quality from execution cost, or for backtests
    /// against genuinely deep, liquid markets.
    None,
    /// A fixed adverse cost in basis points, regardless of order size.
    FixedBps(f64),
    /// Cost grows with order size relative to a reference notional,
    /// modeling thinner liquidity absorbing larger orders worse:
    /// `bps = base_bps + (size_usd / reference_usd) * scale_bps`.
    SizeScaled {
        base_bps: f64,
        reference_usd: f64,
        scale_bps: f64,
    },
}

impl SlippageModel {
    pub fn bps_for(&self, size_usd: u64) -> f64 {
        match *self {
            SlippageModel::None => 0.0,
            SlippageModel::FixedBps(bps) => bps,
            SlippageModel::SizeScaled {
                base_bps,
                reference_usd,
                scale_bps,
            } => {
                if reference_usd <= 0.0 {
                    return base_bps;
                }
                base_bps + (size_usd as f64 / reference_usd) * scale_bps
            }
        }
    }

    /// The execution price a trade of `size_usd` at `mid_price` would fill
    /// at. Buys pay up (`is_buy: true`); sells receive less.
    pub fn execution_price(&self, mid_price: f64, size_usd: u64, is_buy: bool) -> f64 {
        let factor = self.bps_for(size_usd).max(0.0) / 10_000.0;
        if is_buy {
            mid_price * (1.0 + factor)
        } else {
            mid_price * (1.0 - factor)
        }
    }
}

/// A flat proportional fee applied to filled notional, e.g. a DEX swap fee.
#[derive(Debug, Clone, Copy)]
pub struct FeeModel {
    pub bps: f64,
}

impl FeeModel {
    pub fn zero() -> Self {
        FeeModel { bps: 0.0 }
    }

    pub fn fee_for(&self, notional_usd: u64) -> f64 {
        notional_usd as f64 * (self.bps.max(0.0) / 10_000.0)
    }
}

/// Caps how much of a signal's sized order can fill against a single
/// historical tick, so an order much larger than this spreads across
/// several ticks ("partially filled") instead of filling instantly and in
/// full against one bare price point that carries no real depth
/// information to justify that.
#[derive(Debug, Clone, Copy)]
pub struct PartialFillConfig {
    pub max_fill_per_tick_usd: u64,
}

impl PartialFillConfig {
    pub fn unlimited() -> Self {
        PartialFillConfig {
            max_fill_per_tick_usd: u64::MAX,
        }
    }

    pub fn fill_amount(&self, remaining_usd: u64) -> u64 {
        remaining_usd.min(self.max_fill_per_tick_usd)
    }
}

/// Bundles the execution-cost models a simulated backtest fill goes
/// through: how much fills this tick, at what price, and what fee it pays.
#[derive(Debug, Clone, Copy)]
pub struct FillModel {
    pub slippage: SlippageModel,
    pub fee: FeeModel,
    pub partial_fill: PartialFillConfig,
}

impl FillModel {
    /// No slippage, no fee, unlimited fill size — matches the live paper
    /// engine's behavior, for backtests that want to isolate signal
    /// quality from execution cost.
    pub fn ideal() -> Self {
        FillModel {
            slippage: SlippageModel::None,
            fee: FeeModel::zero(),
            partial_fill: PartialFillConfig::unlimited(),
        }
    }

    /// Simulate filling up to `remaining_usd` of an order at `mid_price`.
    /// Returns the `Fill` to record and the USD notional actually filled
    /// (which may be less than `remaining_usd` under the partial-fill cap,
    /// leaving the order `PartiallyFilled` for the next tick to continue).
    pub fn simulate_fill(
        &self,
        remaining_usd: u64,
        mid_price: f64,
        is_buy: bool,
        timestamp: DateTime<Utc>,
    ) -> (Fill, u64) {
        let filled_usd = self.partial_fill.fill_amount(remaining_usd);
        let exec_price = self.slippage.execution_price(mid_price, filled_usd, is_buy);
        let fee = self.fee.fee_for(filled_usd);
        (
            Fill {
                amount: filled_usd,
                price: exec_price,
                fee,
                timestamp,
                tx_signature: None,
            },
            filled_usd,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_none_slippage_is_exact_price() {
        let model = SlippageModel::None;
        assert_eq!(model.execution_price(100.0, 10_000, true), 100.0);
        assert_eq!(model.execution_price(100.0, 10_000, false), 100.0);
    }

    #[test]
    fn test_fixed_bps_slippage_direction() {
        let model = SlippageModel::FixedBps(50.0); // 0.5%
        let buy = model.execution_price(100.0, 10_000, true);
        let sell = model.execution_price(100.0, 10_000, false);
        assert!((buy - 100.5).abs() < 1e-9);
        assert!((sell - 99.5).abs() < 1e-9);
    }

    #[test]
    fn test_size_scaled_slippage_grows_with_size() {
        let model = SlippageModel::SizeScaled {
            base_bps: 5.0,
            reference_usd: 10_000.0,
            scale_bps: 20.0,
        };
        let small = model.bps_for(1_000);
        let large = model.bps_for(50_000);
        assert!(large > small);
        assert!((small - 7.0).abs() < 1e-9); // 5 + (1000/10000)*20 = 7
    }

    #[test]
    fn test_fee_model_proportional() {
        let fee = FeeModel { bps: 25.0 }; // 0.25%
        assert!((fee.fee_for(10_000) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn test_partial_fill_caps_amount() {
        let cfg = PartialFillConfig {
            max_fill_per_tick_usd: 1_000,
        };
        assert_eq!(cfg.fill_amount(5_000), 1_000);
        assert_eq!(cfg.fill_amount(500), 500);
    }

    #[test]
    fn test_simulate_fill_applies_all_three_models() {
        let model = FillModel {
            slippage: SlippageModel::FixedBps(100.0), // 1%
            fee: FeeModel { bps: 25.0 },
            partial_fill: PartialFillConfig {
                max_fill_per_tick_usd: 500,
            },
        };
        let (fill, filled) = model.simulate_fill(2_000, 100.0, true, Utc::now());
        assert_eq!(filled, 500); // capped by partial fill
        assert!((fill.price - 101.0).abs() < 1e-9); // 1% adverse
        assert!((fill.fee - 1.25).abs() < 1e-9); // 0.25% of 500
        assert_eq!(fill.amount, 500);
    }
}
