//! Solstice Storage
//!
//! Persistence layer: PostgreSQL/TimescaleDB for time-series and
//! transactional data, Redis for hot-path caching and pub/sub.

pub mod cache;
pub mod config;
pub mod error;
pub mod models;
pub mod pool;

pub use cache::CacheManager;
pub use config::{PostgresConfig, RedisConfig, StorageConfig};
pub use error::{StorageError, StorageResult};
pub use pool::{StoragePool, TimeRange};

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::StoragePool>;
    }
}
