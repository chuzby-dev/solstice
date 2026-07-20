//! DEX integration error types.

use thiserror::Error;

/// DEX-specific error types.
#[derive(Error, Debug)]
pub enum DexError {
    #[error("HTTP request failed: {0}")]
    Http(String),

    #[error("API error from {dex}: {message}")]
    ApiError { dex: String, message: String },

    #[error("failed to parse response from {dex}: {message}")]
    ParseError { dex: String, message: String },

    #[error("no quote available for this pair/amount")]
    NoQuote,

    #[error("no route found across any configured DEX")]
    NoRoute,

    #[error("unknown DEX: {0}")]
    UnknownDex(String),

    #[error("account query error: {0}")]
    AccountQuery(String),

    #[error("invalid pool state: {0}")]
    InvalidPoolState(String),
}

/// Result type alias for DEX operations.
pub type DexResult<T> = std::result::Result<T, DexError>;

impl From<reqwest::Error> for DexError {
    fn from(err: reqwest::Error) -> Self {
        DexError::Http(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = DexError::UnknownDex("foo".to_string());
        assert_eq!(err.to_string(), "unknown DEX: foo");
    }

    #[test]
    fn test_api_error_display() {
        let err = DexError::ApiError {
            dex: "Jupiter".to_string(),
            message: "rate limited".to_string(),
        };
        assert!(err.to_string().contains("Jupiter"));
        assert!(err.to_string().contains("rate limited"));
    }
}
