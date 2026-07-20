//! Integration tests requiring live Postgres/Redis instances.
//!
//! These are `#[ignore]`d by default since this environment has neither
//! running. To exercise them:
//!
//! ```sh
//! docker run -d --name solstice-pg -p 5432:5432 -e POSTGRES_PASSWORD=postgres timescale/timescaledb:latest-pg16
//! docker run -d --name solstice-redis -p 6379:6379 redis:7
//! STORAGE_TEST_POSTGRES_URL=postgres://postgres:postgres@localhost/postgres \
//! STORAGE_TEST_REDIS_URL=redis://localhost \
//!   cargo test -p solstice-storage -- --ignored
//! ```

use chrono::Utc;
use solana_sdk::pubkey::Pubkey;
use solstice_core::types::{Position, PositionId, Trade, TradeAction};
use solstice_storage::config::{PostgresConfig, RedisConfig, StorageConfig};
use solstice_storage::pool::TimeRange;
use solstice_storage::{CacheManager, StoragePool};
use std::time::Duration;

fn test_config() -> StorageConfig {
    let postgres_url = std::env::var("STORAGE_TEST_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/postgres".to_string());
    let redis_url =
        std::env::var("STORAGE_TEST_REDIS_URL").unwrap_or_else(|_| "redis://localhost".to_string());

    StorageConfig {
        postgres: PostgresConfig::new(postgres_url).with_max_connections(5),
        redis: RedisConfig::new(redis_url).with_key_prefix("solstice-test:"),
    }
}

#[tokio::test]
#[ignore = "requires a live Postgres/TimescaleDB instance"]
async fn test_storage_pool_migrations_and_trade_roundtrip() {
    let config = test_config();
    let storage = StoragePool::new(&config).await.expect("connect+migrate");

    let pair = solstice_core::types::TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
    let trade = Trade::new(PositionId::new(), pair, TradeAction::Buy, 1_000, 50.0, 1.0);

    storage.save_trade(&trade).await.expect("save trade");

    let history = storage
        .get_market_data(
            &pair.base,
            &pair.quote,
            TimeRange::last(chrono::Duration::hours(1)),
        )
        .await
        .expect("query market data");
    assert!(history.is_empty());
}

#[tokio::test]
#[ignore = "requires a live Postgres/TimescaleDB instance"]
async fn test_position_history_roundtrip() {
    let config = test_config();
    let storage = StoragePool::new(&config).await.expect("connect+migrate");

    let pair = solstice_core::types::TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
    let position = Position::new(pair, 100, 10.0);

    storage
        .save_position_update(&position)
        .await
        .expect("save position update");

    let history = storage
        .get_position_history(position.id)
        .await
        .expect("query position history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].quantity, 100);
}

#[tokio::test]
#[ignore = "requires a live Postgres/TimescaleDB instance"]
async fn test_account_snapshot_roundtrip() {
    let config = test_config();
    let storage = StoragePool::new(&config).await.expect("connect+migrate");

    let address = Pubkey::new_unique();
    let row = solstice_storage::models::AccountSnapshotRow::new(
        address,
        Pubkey::new_unique(),
        1_000_000,
        vec![1, 2, 3],
        1,
        Utc::now(),
    )
    .unwrap();

    storage
        .save_account_snapshot(&row)
        .await
        .expect("save snapshot");

    let latest = storage
        .get_latest_account_snapshot(&address)
        .await
        .expect("query snapshot")
        .expect("snapshot present");
    assert_eq!(latest.lamports, 1_000_000);
}

#[tokio::test]
#[ignore = "requires a live Redis instance"]
async fn test_cache_manager_get_set_delete() {
    let config = test_config();
    let cache = CacheManager::new(config.redis).await.expect("connect");

    cache
        .set("roundtrip", b"hello".to_vec(), Duration::from_secs(30))
        .await
        .expect("set");

    let value = cache.get("roundtrip").await.expect("get");
    assert_eq!(value, Some(b"hello".to_vec()));

    let removed = cache.delete("roundtrip").await.expect("delete");
    assert!(removed);

    let value = cache.get("roundtrip").await.expect("get after delete");
    assert_eq!(value, None);
}

#[tokio::test]
#[ignore = "requires a live Redis instance"]
async fn test_cache_manager_json_roundtrip() {
    let config = test_config();
    let cache = CacheManager::new(config.redis).await.expect("connect");

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Sample {
        value: u64,
    }

    let sample = Sample { value: 42 };
    cache
        .set_json("json-roundtrip", &sample, Duration::from_secs(30))
        .await
        .expect("set_json");

    let fetched: Option<Sample> = cache.get_json("json-roundtrip").await.expect("get_json");
    assert_eq!(fetched, Some(sample));

    cache.delete("json-roundtrip").await.expect("cleanup");
}
