//! Solstice Execution & Risk
//!
//! Position sizing (fractional Kelly), hard risk limits and monitoring,
//! stop-loss evaluation, pre-trade risk checks, execution planning
//! (routing via `solstice-dex`), and order lifecycle tracking. See
//! `docs/RISK_MANAGEMENT.md` and `docs/WORKSPACE.md`'s `solstice-execution`
//! summary.

pub mod error;
pub mod order_manager;
pub mod planner;
pub mod position_sizing;
pub mod risk;

pub use error::{ExecutionError, ExecutionResult};
pub use order_manager::{Fill, Order, OrderManager, OrderStatus};
pub use planner::{signal_pair, ExecutionPlan, ExecutionPlanner, PortfolioContext};
pub use position_sizing::{PositionSizer, RiskParams};
pub use risk::{
    ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PortfolioRiskMetrics,
    PositionLimits, PreTradeRiskChecker, RiskLimitStatus, RiskLimits, RiskMonitor, StopLossManager,
    StopLossTrigger, TradeApproval,
};

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::OrderManager>;
    }
}
