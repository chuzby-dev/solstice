//! Execution and risk management error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ExecutionError {
    #[error("risk limit violated: {0}")]
    RiskLimitViolated(String),

    #[error("no route available for this trade")]
    NoRoute,

    #[error("position sizing failed: {0}")]
    SizingFailed(String),

    #[error("order not found: {0}")]
    OrderNotFound(String),

    #[error("invalid order state transition: {0}")]
    InvalidOrderTransition(String),

    #[error("DEX error: {0}")]
    Dex(#[from] solstice_dex::DexError),

    #[error("failed to build swap transaction: {0}")]
    TransactionBuildFailed(String),
}

pub type ExecutionResult<T> = std::result::Result<T, ExecutionError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ExecutionError::RiskLimitViolated("daily loss".to_string());
        assert!(err.to_string().contains("daily loss"));
    }
}
