//! Redis-backed cache manager.

use crate::config::RedisConfig;
use crate::error::{StorageError, StorageResult};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;

/// Redis cache manager. Cheap to clone: it holds a multiplexed,
/// auto-reconnecting connection internally.
#[derive(Clone)]
pub struct CacheManager {
    conn: ConnectionManager,
    config: RedisConfig,
}

impl CacheManager {
    pub async fn new(config: RedisConfig) -> StorageResult<Self> {
        let client = redis::Client::open(config.url.clone())
            .map_err(|e| StorageError::CacheConnection(e.to_string()))?;
        let conn = client
            .get_connection_manager()
            .await
            .map_err(|e| StorageError::CacheConnection(e.to_string()))?;

        Ok(CacheManager { conn, config })
    }

    /// Get a raw value by key, `None` if absent or expired.
    pub async fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let value: Option<Vec<u8>> = self.conn.clone().get(self.config.prefixed_key(key)).await?;
        Ok(value)
    }

    /// Set a raw value with an explicit TTL.
    pub async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> StorageResult<()> {
        let seconds = ttl.as_secs().max(1);
        self.conn
            .clone()
            .set_ex::<_, _, ()>(self.config.prefixed_key(key), value, seconds)
            .await?;
        Ok(())
    }

    /// Set a raw value using the configured default TTL.
    pub async fn set_default_ttl(&self, key: &str, value: Vec<u8>) -> StorageResult<()> {
        self.set(key, value, self.config.default_ttl).await
    }

    /// Delete a key. Returns whether a key was actually removed.
    pub async fn delete(&self, key: &str) -> StorageResult<bool> {
        let removed: i64 = self.conn.clone().del(self.config.prefixed_key(key)).await?;
        Ok(removed > 0)
    }

    pub async fn exists(&self, key: &str) -> StorageResult<bool> {
        let exists: bool = self
            .conn
            .clone()
            .exists(self.config.prefixed_key(key))
            .await?;
        Ok(exists)
    }

    /// Get and JSON-deserialize a value.
    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> StorageResult<Option<T>> {
        match self.get(key).await? {
            Some(bytes) => {
                let value = serde_json::from_slice(&bytes)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// JSON-serialize and set a value with an explicit TTL.
    pub async fn set_json<T: Serialize + Sync>(
        &self,
        key: &str,
        value: &T,
        ttl: Duration,
    ) -> StorageResult<()> {
        let bytes =
            serde_json::to_vec(value).map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.set(key, bytes, ttl).await
    }

    /// Publish a message on a pub/sub channel (not prefixed, since channels
    /// are a separate namespace from keys).
    pub async fn publish(&self, channel: &str, message: &[u8]) -> StorageResult<()> {
        self.conn
            .clone()
            .publish::<_, _, ()>(channel, message)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_config_default_ttl() {
        let config =
            RedisConfig::new("redis://localhost").with_default_ttl(Duration::from_secs(30));
        assert_eq!(config.default_ttl, Duration::from_secs(30));
    }

    // Connection-requiring behavior (get/set/delete/publish round trips) is
    // covered by tests/integration_tests.rs, which is #[ignore]'d pending a
    // live Redis instance; see that file for how to run it.
}
