//! Storage layer error types.

use thiserror::Error;

/// Storage-specific error types.
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("database connection error: {0}")]
    Connection(String),

    #[error("database query error: {0}")]
    Query(String),

    #[error("migration error: {0}")]
    Migration(String),

    #[error("cache connection error: {0}")]
    CacheConnection(String),

    #[error("cache operation error: {0}")]
    CacheOperation(String),

    #[error("value {value} out of range for column {column}")]
    ValueOutOfRange { column: &'static str, value: String },

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("record not found")]
    NotFound,
}

impl From<sqlx::Error> for StorageError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => StorageError::NotFound,
            other => StorageError::Query(other.to_string()),
        }
    }
}

impl From<sqlx::migrate::MigrateError> for StorageError {
    fn from(err: sqlx::migrate::MigrateError) -> Self {
        StorageError::Migration(err.to_string())
    }
}

impl From<redis::RedisError> for StorageError {
    fn from(err: redis::RedisError) -> Self {
        StorageError::CacheOperation(err.to_string())
    }
}

/// Result type alias for storage operations.
pub type StorageResult<T> = std::result::Result<T, StorageError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = StorageError::NotFound;
        assert_eq!(err.to_string(), "record not found");
    }

    #[test]
    fn test_value_out_of_range_display() {
        let err = StorageError::ValueOutOfRange {
            column: "quantity",
            value: "999999999999999999999".to_string(),
        };
        assert!(err.to_string().contains("quantity"));
    }

    #[test]
    fn test_from_sqlx_row_not_found() {
        let err: StorageError = sqlx::Error::RowNotFound.into();
        assert!(matches!(err, StorageError::NotFound));
    }
}
