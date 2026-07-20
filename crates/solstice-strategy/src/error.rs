//! Strategy framework error types.

use thiserror::Error;

/// Strategy-specific error types.
#[derive(Error, Debug)]
pub enum StrategyError {
    #[error("invalid strategy configuration: {0}")]
    InvalidConfig(String),

    #[error("strategy evaluation failed: {0}")]
    EvaluationFailed(String),

    #[error("strategy '{0}' not found")]
    NotFound(String),

    #[error("strategy '{0}' already registered")]
    AlreadyRegistered(String),

    #[error("invalid signal: {0}")]
    InvalidSignal(String),
}

/// Result type alias for strategy operations.
pub type StrategyResult<T> = std::result::Result<T, StrategyError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = StrategyError::NotFound("SMA".to_string());
        assert_eq!(err.to_string(), "strategy 'SMA' not found");
    }
}
