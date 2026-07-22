//! Solstice Execution & Risk
//!
//! Position sizing (fractional Kelly), hard risk limits and monitoring,
//! stop-loss evaluation, pre-trade risk checks, execution planning
//! (routing via `solstice-dex`), and order lifecycle tracking. See
//! `docs/RISK_MANAGEMENT.md` and `docs/WORKSPACE.md`'s `solstice-execution`
//! summary.

pub mod error;
pub mod jito;
pub mod live;
pub mod order_manager;
pub mod planner;
pub mod position_sizing;
pub mod risk;
pub mod swap;

pub use error::{ExecutionError, ExecutionResult};
pub use live::{
    LiveEvent, LiveStatusSnapshot, LiveTradedPair, LiveTradingConfig, LiveTradingEngine,
};
pub use order_manager::{Fill, Order, OrderManager, OrderStatus};
pub use planner::{signal_pair, ExecutionPlan, ExecutionPlanner, PortfolioContext};
pub use position_sizing::{PositionSizer, RiskParams};
pub use risk::{
    ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PortfolioRiskMetrics,
    PositionLimits, PreTradeRiskChecker, RiskLimitStatus, RiskLimits, RiskMonitor, StopLossManager,
    StopLossTrigger, TradeApproval,
};
pub use swap::{
    build_atomic_swap_transaction, build_swap_transaction, execute_atomic_arb, execute_swap,
    MAX_TRANSACTION_SIZE,
};

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::OrderManager>;
    }
}
