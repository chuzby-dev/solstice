//! Error types and Result aliases for Solstice.

use thiserror::Error;

/// Solstice error types.
#[derive(Error, Debug)]
pub enum SolsticeError {
    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Market data error: {0}")]
    MarketDataError(String),

    #[error("Blockchain error: {0}")]
    BlockchainError(String),

    #[error("DEX error: {0}")]
    DexError(String),

    #[error("Strategy error: {0}")]
    StrategyError(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("Risk management error: {0}")]
    RiskError(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Simulation error: {0}")]
    SimulationError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

/// Result type alias for Solstice operations.
pub type Result<T> = std::result::Result<T, SolsticeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SolsticeError::ConfigError("invalid config".to_string());
        assert!(err.to_string().contains("Configuration error"));
    }

    #[test]
    fn test_result_type() {
        let ok: Result<i32> = Ok(42);
        match ok {
            Ok(value) => assert_eq!(value, 42),
            Err(_) => panic!("expected Ok"),
        }

        let err: Result<i32> = Err(SolsticeError::NotFound("test".to_string()));
        assert!(err.is_err());
    }
}
