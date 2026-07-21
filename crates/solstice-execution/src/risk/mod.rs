//! Risk management: hard limits, monitoring, stop-loss, and pre-trade checks.

pub mod limits;
pub mod monitor;
pub mod pretrade;
pub mod stop_loss;
pub mod take_profit;

pub use limits::{
    ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits, RiskLimits,
};
pub use monitor::{PortfolioRiskMetrics, RiskLimitStatus, RiskMonitor};
pub use pretrade::{PreTradeRiskChecker, TradeApproval};
pub use stop_loss::{StopLossManager, StopLossTrigger};
pub use take_profit::{TakeProfitManager, TakeProfitTrigger};
