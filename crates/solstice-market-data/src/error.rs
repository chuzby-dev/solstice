//! Market data error types.

use thiserror::Error;

/// Market data-specific error types.
#[derive(Error, Debug)]
pub enum MarketDataError {
    #[error("Ingestion error: {0}")]
    IngestionError(String),

    #[error("Normalization error: {0}")]
    NormalizationError(String),

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("Invalid market data: {0}")]
    InvalidData(String),

    #[error("Subscription error: {0}")]
    SubscriptionError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("No data available")]
    NoData,

    #[error("Data stale (age: {0}s)")]
    StaleData(u64),
}

/// Result type alias for market data operations.
pub type MarketDataResult<T> = std::result::Result<T, MarketDataError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = MarketDataError::InvalidData("test".to_string());
        assert!(err.to_string().contains("Invalid market data"));
    }
}
