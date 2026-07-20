//! Storage connection and pooling configuration.

use std::time::Duration;

/// Configuration for the PostgreSQL/TimescaleDB connection pool.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout: Duration,
    pub idle_timeout: Option<Duration>,
    /// Run pending migrations on `StoragePool::new`.
    pub run_migrations_on_startup: bool,
}

impl PostgresConfig {
    pub fn new(url: impl Into<String>) -> Self {
        PostgresConfig {
            url: url.into(),
            max_connections: 20,
            min_connections: 2,
            connect_timeout: Duration::from_secs(10),
            idle_timeout: Some(Duration::from_secs(600)),
            run_migrations_on_startup: true,
        }
    }

    pub fn with_max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    pub fn with_run_migrations_on_startup(mut self, run: bool) -> Self {
        self.run_migrations_on_startup = run;
        self
    }
}

/// Configuration for the Redis cache connection.
#[derive(Debug, Clone)]
pub struct RedisConfig {
    pub url: String,
    /// Default TTL applied when a caller doesn't specify one explicitly.
    pub default_ttl: Duration,
    /// Prefix prepended to every key this instance manages, so multiple
    /// environments (or the platform vs. tests) can safely share one Redis.
    pub key_prefix: String,
}

impl RedisConfig {
    pub fn new(url: impl Into<String>) -> Self {
        RedisConfig {
            url: url.into(),
            default_ttl: Duration::from_secs(60),
            key_prefix: "solstice:".to_string(),
        }
    }

    pub fn with_default_ttl(mut self, ttl: Duration) -> Self {
        self.default_ttl = ttl;
        self
    }

    pub fn with_key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    pub fn prefixed_key(&self, key: &str) -> String {
        format!("{}{}", self.key_prefix, key)
    }
}

/// Combined storage configuration for [`crate::pool::StoragePool`].
#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub postgres: PostgresConfig,
    pub redis: RedisConfig,
}

impl StorageConfig {
    pub fn new(postgres_url: impl Into<String>, redis_url: impl Into<String>) -> Self {
        StorageConfig {
            postgres: PostgresConfig::new(postgres_url),
            redis: RedisConfig::new(redis_url),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postgres_config_defaults() {
        let config = PostgresConfig::new("postgres://localhost/solstice");
        assert_eq!(config.max_connections, 20);
        assert!(config.run_migrations_on_startup);
    }

    #[test]
    fn test_redis_prefixed_key() {
        let config = RedisConfig::new("redis://localhost").with_key_prefix("test:");
        assert_eq!(config.prefixed_key("price:SOL"), "test:price:SOL");
    }

    #[test]
    fn test_postgres_builder_methods() {
        let config = PostgresConfig::new("postgres://localhost/solstice")
            .with_max_connections(5)
            .with_run_migrations_on_startup(false);

        assert_eq!(config.max_connections, 5);
        assert!(!config.run_migrations_on_startup);
    }
}
