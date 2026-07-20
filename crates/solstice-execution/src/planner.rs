//! Execution planning: turning an approved signal into a priced,
//! risk-checked plan ready for order submission.

use crate::error::{ExecutionError, ExecutionResult};
use crate::risk::{PreTradeRiskChecker, RiskLimits, TradeApproval};
use solstice_core::types::{Signal, SignalType, TokenPair};
use solstice_dex::{DexAggregator, Quote, QuoteRequest};
use std::sync::Arc;

/// A priced, risk-checked execution plan. `approval` must be checked
/// before acting on `quote` — a rejected plan is still returned (not an
/// error) so callers can inspect/log why.
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub signal: Signal,
    pub pair: TokenPair,
    pub quote: Quote,
    pub size_usd: u64,
    pub approval: TradeApproval,
}

/// Extracts the token pair a signal concerns, if any. `Close`/`Rebalance`
/// signals don't concern a single pair the same way `Buy`/`Sell` do, so
/// they have no execution plan through this path.
pub fn signal_pair(signal: &Signal) -> Option<TokenPair> {
    match signal.signal_type {
        SignalType::Buy { pair } | SignalType::Sell { pair } => Some(pair),
        SignalType::Close { .. } | SignalType::Rebalance { .. } => None,
    }
}

/// Portfolio context a plan is evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct PortfolioContext {
    pub portfolio_value_usd: u64,
    pub current_open_positions: usize,
    pub current_exposure_usd: u64,
    pub daily_pnl_usd: i64,
}

pub struct ExecutionPlanner {
    aggregator: Arc<DexAggregator>,
    risk_checker: PreTradeRiskChecker,
    default_slippage_bps: u32,
}

impl ExecutionPlanner {
    pub fn new(
        aggregator: Arc<DexAggregator>,
        limits: RiskLimits,
        default_slippage_bps: u32,
    ) -> Self {
        ExecutionPlanner {
            aggregator,
            risk_checker: PreTradeRiskChecker::new(limits),
            default_slippage_bps,
        }
    }

    /// Plan execution for `signal`: fetch the best available route for
    /// `size_usd` worth of the signal's pair, estimate slippage, and run
    /// pre-trade risk checks. Returns `Err` only for infrastructure
    /// failures (no route found, DEX error) — a plan that fails risk
    /// checks is still `Ok`, with `approval: Rejected`.
    pub async fn plan(
        &self,
        signal: &Signal,
        size_usd: u64,
        context: PortfolioContext,
    ) -> ExecutionResult<ExecutionPlan> {
        let pair = signal_pair(signal).ok_or_else(|| {
            ExecutionError::SizingFailed(
                "signal type has no associated pair to plan a swap for".to_string(),
            )
        })?;

        let (input_mint, output_mint) = match signal.signal_type {
            SignalType::Buy { .. } => (pair.quote, pair.base),
            SignalType::Sell { .. } => (pair.base, pair.quote),
            _ => unreachable!("signal_pair already filtered to Buy/Sell"),
        };

        let request =
            QuoteRequest::new(input_mint, output_mint, size_usd, self.default_slippage_bps);

        let quote = self
            .aggregator
            .get_best_route(&request)
            .await
            .map_err(ExecutionError::from)?;

        let slippage = self
            .aggregator
            .estimate_slippage(&request)
            .await
            .unwrap_or(quote.price_impact);

        let approval = self.risk_checker.check_before_trade(
            size_usd,
            context.portfolio_value_usd,
            context.current_open_positions,
            context.current_exposure_usd,
            context.daily_pnl_usd,
            Some(slippage),
        );

        Ok(ExecutionPlan {
            signal: signal.clone(),
            pair,
            quote,
            size_usd,
            approval,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn test_signal_pair_buy() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let signal = Signal::new("test".to_string(), SignalType::Buy { pair }, 0.9);
        assert_eq!(signal_pair(&signal), Some(pair));
    }

    #[test]
    fn test_signal_pair_rebalance_has_none() {
        let signal = Signal::new(
            "test".to_string(),
            SignalType::Rebalance {
                reason: "drift".to_string(),
            },
            0.9,
        );
        assert_eq!(signal_pair(&signal), None);
    }
}
