//! Jito-specific error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum JitoError {
    #[error("bundle exceeds Jito's max size of {max} transactions (got {actual})")]
    BundleTooLarge { max: usize, actual: usize },

    #[error("bundle is empty")]
    EmptyBundle,

    #[error("HTTP request to Jito Block Engine failed: {0}")]
    Http(String),

    #[error("Jito Block Engine returned an error {code}: {message}")]
    Rpc { code: i64, message: String },

    #[error("unexpected response from Jito Block Engine: {0}")]
    InvalidResponse(String),

    #[error("no configured Jito Block Engine endpoint accepted the bundle: {0}")]
    AllEndpointsFailed(String),

    #[error("timed out waiting for bundle confirmation")]
    ConfirmationTimeout,

    #[error("transaction serialization failed: {0}")]
    Serialization(String),

    #[error("direct RPC submission failed after Jito fallback: {0}")]
    DirectSubmissionFailed(String),
}

pub type JitoResult<T> = std::result::Result<T, JitoError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = JitoError::BundleTooLarge { max: 5, actual: 6 };
        assert!(err.to_string().contains("max size of 5"));
    }
}
