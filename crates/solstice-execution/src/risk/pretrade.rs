//! Pre-trade risk checks: the last gate before a signal becomes an order.
//!
//! Unlike `docs/RISK_MANAGEMENT.md`'s sketch (which has the checker fetch
//! a quote from a `DexAggregator` itself), `simulated_slippage` is passed
//! in by the caller. Risk checks are pure/synchronous here; fetching a
//! quote is an I/O concern that belongs to whatever's orchestrating the
//! trade (e.g. [`crate::planner::ExecutionPlanner`]), not to the risk
//! checker itself.

use crate::risk::limits::RiskLimits;

#[derive(Debug, Clone, PartialEq)]
pub enum TradeApproval {
    Approved,
    Rejected { reason: String },
}

impl TradeApproval {
    pub fn is_approved(&self) -> bool {
        matches!(self, TradeApproval::Approved)
    }
}

pub struct PreTradeRiskChecker {
    limits: RiskLimits,
}

impl PreTradeRiskChecker {
    pub fn new(limits: RiskLimits) -> Self {
        PreTradeRiskChecker { limits }
    }

    /// Run every configured risk check in order, returning the first
    /// rejection encountered (or `Approved` if all pass).
    #[allow(clippy::too_many_arguments)]
    pub fn check_before_trade(
        &self,
        position_size_usd: u64,
        portfolio_value_usd: u64,
        current_open_positions: usize,
        current_exposure_usd: u64,
        daily_pnl_usd: i64,
        simulated_slippage: Option<f64>,
    ) -> TradeApproval {
        if let Err(e) = self.limits.position.can_open(
            portfolio_value_usd,
            position_size_usd,
            current_open_positions,
        ) {
            return TradeApproval::Rejected {
                reason: e.to_string(),
            };
        }

        if let Err(e) = self.limits.exposure.can_increase_exposure(
            current_exposure_usd,
            position_size_usd,
            portfolio_value_usd,
        ) {
            return TradeApproval::Rejected {
                reason: e.to_string(),
            };
        }

        if let Err(e) = self
            .limits
            .concentration
            .check_concentration(position_size_usd, portfolio_value_usd)
        {
            return TradeApproval::Rejected {
                reason: e.to_string(),
            };
        }

        if daily_pnl_usd < 0 {
            if let Err(e) = self
                .limits
                .daily_loss
                .check_loss(daily_pnl_usd, portfolio_value_usd)
            {
                return TradeApproval::Rejected {
                    reason: e.to_string(),
                };
            }
        }

        if let Some(slippage) = simulated_slippage {
            if let Err(e) = self
                .limits
                .order
                .can_submit_order(position_size_usd, slippage)
            {
                return TradeApproval::Rejected {
                    reason: e.to_string(),
                };
            }
        }

        TradeApproval::Approved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::limits::{
        ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits,
    };

    fn limits() -> RiskLimits {
        RiskLimits {
            position: PositionLimits {
                max_single_position_usd: 100_000,
                max_position_percent: 0.25,
                min_position_size_usd: 1_000,
                max_open_positions: 50,
            },
            daily_loss: DailyLossLimits {
                max_daily_loss_usd: 50_000,
                max_daily_loss_percent: 0.5,
            },
            exposure: ExposureLimits {
                max_total_exposure_usd: 500_000,
                max_leverage: 1.0,
            },
            concentration: ConcentrationLimits {
                max_single_asset_percent: 0.3,
            },
            order: OrderLimits {
                max_order_size_usd: 50_000,
                max_slippage_percent: 0.02,
            },
        }
    }

    #[test]
    fn test_approved_when_all_checks_pass() {
        let checker = PreTradeRiskChecker::new(limits());
        let approval =
            checker.check_before_trade(10_000, 1_000_000, 5, 100_000, 1_000, Some(0.005));
        assert!(approval.is_approved());
    }

    #[test]
    fn test_rejected_on_position_limit() {
        let checker = PreTradeRiskChecker::new(limits());
        let approval = checker.check_before_trade(500, 1_000_000, 5, 100_000, 1_000, None);
        assert!(!approval.is_approved());
    }

    #[test]
    fn test_rejected_on_slippage() {
        let checker = PreTradeRiskChecker::new(limits());
        let approval = checker.check_before_trade(10_000, 1_000_000, 5, 100_000, 1_000, Some(0.05));
        assert!(!approval.is_approved());
    }

    #[test]
    fn test_rejected_on_daily_loss() {
        let checker = PreTradeRiskChecker::new(limits());
        let approval = checker.check_before_trade(10_000, 1_000_000, 5, 100_000, -60_000, None);
        assert!(!approval.is_approved());
    }

    #[test]
    fn test_no_slippage_check_when_not_provided() {
        let checker = PreTradeRiskChecker::new(limits());
        let approval = checker.check_before_trade(10_000, 1_000_000, 5, 100_000, 1_000, None);
        assert!(approval.is_approved());
    }
}
