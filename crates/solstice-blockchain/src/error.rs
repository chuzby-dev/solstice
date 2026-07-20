//! Blockchain-specific error types.

use thiserror::Error;

/// Blockchain-specific error types.
#[derive(Error, Debug)]
pub enum BlockchainError {
    #[error("RPC error: {0}")]
    RpcError(String),

    #[error("Account not found: {0}")]
    AccountNotFound(String),

    #[error("Invalid account data")]
    InvalidAccountData,

    #[error("Transaction error: {0}")]
    TransactionError(String),

    #[error("Transaction failed: {0}")]
    TransactionFailed(String),

    #[error("Transaction timeout")]
    TransactionTimeout,

    #[error("Signature invalid: {0}")]
    InvalidSignature(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("All RPC endpoints failed")]
    AllEndpointsFailed,

    #[error("No endpoints configured")]
    NoEndpoints,

    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("Slot error: {0}")]
    SlotError(String),
}

/// Result type alias for blockchain operations.
pub type BlockchainResult<T> = std::result::Result<T, BlockchainError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = BlockchainError::AccountNotFound("test_account".to_string());
        assert!(err.to_string().contains("Account not found"));
    }
}
