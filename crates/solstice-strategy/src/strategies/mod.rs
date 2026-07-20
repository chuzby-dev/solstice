//! Reference strategy implementations.

pub mod sma;
pub mod spread_arb;

pub use sma::SimpleMovingAverageStrategy;
pub use spread_arb::SpreadArbitrageStrategy;
