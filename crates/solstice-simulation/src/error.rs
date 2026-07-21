//! Simulation error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SimulationError {
    #[error("DEX error: {0}")]
    Dex(#[from] solstice_dex::DexError),

    #[error("execution error: {0}")]
    Execution(#[from] solstice_execution::ExecutionError),

    #[error("strategy error: {0}")]
    Strategy(#[from] solstice_strategy::StrategyError),

    #[error("no price available for pair")]
    NoPrice,

    #[error("historical data error: {0}")]
    HistoricalData(String),
}

pub type SimulationResult<T> = std::result::Result<T, SimulationError>;
