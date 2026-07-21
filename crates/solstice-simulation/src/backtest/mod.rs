//! Phase 6.1/6.2/6.4: historical-replay simulation, simulated order
//! execution (slippage/partial fills/fees), and a backtesting engine with
//! performance reporting and parameter sweeps.
//!
//! This is deliberately a separate engine from [`crate::engine::PaperTradingEngine`]
//! rather than a generalization of it: the live engine polls real DEX
//! quotes and fills at that quote's exact price (there is no slippage/fee
//! model to speak of, because the quote already reflects real venue
//! conditions). A backtest replays bare historical price points with none
//! of that context, so it needs its own execution-cost modeling
//! ([`fill_model`]) to avoid overstating strategy performance with
//! free, instant, infinite-size fills.

pub mod data;
pub mod engine;
pub mod fill_model;
pub mod optimize;
pub mod report;

pub use data::{load_csv, HistoricalTick};
pub use engine::{BacktestConfig, BacktestEngine};
pub use fill_model::{FeeModel, FillModel, PartialFillConfig, SlippageModel};
pub use optimize::{cartesian_product, optimize_grid, ParameterCandidate, SweepResult};
pub use report::{
    BacktestReport, ClosedPositionRecord, EquityPoint, PerformanceMetrics, TradeRecord,
};
