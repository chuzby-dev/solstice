//! Simulation error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SimulationError {
    #[error("DEX error: {0}")]
    Dex(#[from] solstice_dex::DexError),

    #[error("execution error: {0}")]
    Execution(#[from] solstice_execution::ExecutionError),

    #[error("no price available for pair")]
    NoPrice,
}

pub type SimulationResult<T> = std::result::Result<T, SimulationError>;
