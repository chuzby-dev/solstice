//! PostgreSQL/TimescaleDB connection pool and typed query interface.

use crate::config::StorageConfig;
use crate::error::{StorageError, StorageResult};
use crate::models::{AccountSnapshotRow, MarketSnapshotRow, PositionUpdateRow, TradeRow};
use chrono::{DateTime, Utc};
use solana_sdk::pubkey::Pubkey;
use solstice_core::types::{Position, PositionId, Trade};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

/// An inclusive time range used to scope time-series queries.
#[derive(Debug, Clone, Copy)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        TimeRange { start, end }
    }

    pub fn last(duration: chrono::Duration) -> Self {
        let end = Utc::now();
        TimeRange {
            start: end - duration,
            end,
        }
    }
}

/// Postgres/TimescaleDB connection pool and typed query interface.
pub struct StoragePool {
    pool: PgPool,
}

impl StoragePool {
    /// Connect to Postgres and, if configured, run pending migrations.
    pub async fn new(config: &StorageConfig) -> StorageResult<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(config.postgres.max_connections)
            .min_connections(config.postgres.min_connections)
            .acquire_timeout(config.postgres.connect_timeout)
            .idle_timeout(config.postgres.idle_timeout)
            .connect(&config.postgres.url)
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        let storage = StoragePool { pool };

        if config.postgres.run_migrations_on_startup {
            storage.run_migrations().await?;
        }

        Ok(storage)
    }

    /// Wrap an already-connected pool (mainly for tests against an
    /// externally managed database).
    pub fn from_pool(pool: PgPool) -> Self {
        StoragePool { pool }
    }

    pub async fn run_migrations(&self) -> StorageResult<()> {
        info!("Running storage migrations");
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    /// Persist a market price observation.
    pub async fn save_market_snapshot(&self, row: &MarketSnapshotRow) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO market_snapshots (time, base_mint, quote_mint, price, confidence, source)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (time, base_mint, quote_mint, source) DO NOTHING
            "#,
        )
        .bind(row.time)
        .bind(&row.base_mint)
        .bind(&row.quote_mint)
        .bind(row.price)
        .bind(row.confidence)
        .bind(&row.source)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Fetch market price history for a token pair within a time range,
    /// ordered oldest to newest.
    pub async fn get_market_data(
        &self,
        base_mint: &Pubkey,
        quote_mint: &Pubkey,
        range: TimeRange,
    ) -> StorageResult<Vec<MarketSnapshotRow>> {
        let rows = sqlx::query_as::<_, MarketSnapshotRow>(
            r#"
            SELECT time, base_mint, quote_mint, price, confidence, source
            FROM market_snapshots
            WHERE base_mint = $1 AND quote_mint = $2 AND time BETWEEN $3 AND $4
            ORDER BY time ASC
            "#,
        )
        .bind(base_mint.to_string())
        .bind(quote_mint.to_string())
        .bind(range.start)
        .bind(range.end)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Persist a completed trade.
    pub async fn save_trade(&self, trade: &Trade) -> StorageResult<()> {
        let row = TradeRow::from_trade(trade)?;

        sqlx::query(
            r#"
            INSERT INTO trades
                (id, position_id, base_mint, quote_mint, action, quantity,
                 execution_price, fees, executed_at, tx_signature)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(&row.id)
        .bind(row.position_id)
        .bind(&row.base_mint)
        .bind(&row.quote_mint)
        .bind(&row.action)
        .bind(row.quantity)
        .bind(row.execution_price)
        .bind(row.fees)
        .bind(row.executed_at)
        .bind(&row.tx_signature)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Record a position state update (call whenever a position is opened,
    /// re-priced, or closed).
    pub async fn save_position_update(&self, position: &Position) -> StorageResult<()> {
        let row = PositionUpdateRow::from_position(position);

        sqlx::query(
            r#"
            INSERT INTO position_updates
                (position_id, base_mint, quote_mint, quantity, entry_price,
                 current_price, opened_at, close_at, recorded_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(row.position_id)
        .bind(&row.base_mint)
        .bind(&row.quote_mint)
        .bind(row.quantity)
        .bind(row.entry_price)
        .bind(row.current_price)
        .bind(row.opened_at)
        .bind(row.close_at)
        .bind(row.recorded_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Fetch the recorded update history for a position, oldest to newest.
    pub async fn get_position_history(
        &self,
        id: PositionId,
    ) -> StorageResult<Vec<PositionUpdateRow>> {
        let position_id: Uuid = id.0;
        let rows = sqlx::query_as::<_, PositionUpdateRow>(
            r#"
            SELECT position_id, base_mint, quote_mint, quantity, entry_price,
                   current_price, opened_at, close_at, recorded_at
            FROM position_updates
            WHERE position_id = $1
            ORDER BY recorded_at ASC
            "#,
        )
        .bind(position_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Persist a raw account state snapshot (e.g. from the Yellowstone adapter).
    pub async fn save_account_snapshot(&self, row: &AccountSnapshotRow) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO account_snapshots (time, address, owner, lamports, data, slot)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (time, address) DO NOTHING
            "#,
        )
        .bind(row.time)
        .bind(&row.address)
        .bind(&row.owner)
        .bind(row.lamports)
        .bind(&row.data)
        .bind(row.slot)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// The most recent account snapshot for an address, if any.
    pub async fn get_latest_account_snapshot(
        &self,
        address: &Pubkey,
    ) -> StorageResult<Option<AccountSnapshotRow>> {
        let row = sqlx::query_as::<_, AccountSnapshotRow>(
            r#"
            SELECT time, address, owner, lamports, data, slot
            FROM account_snapshots
            WHERE address = $1
            ORDER BY time DESC
            LIMIT 1
            "#,
        )
        .bind(address.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_range_last() {
        let range = TimeRange::last(chrono::Duration::hours(1));
        assert!(range.end > range.start);
        assert_eq!((range.end - range.start).num_minutes(), 60);
    }

    #[test]
    fn test_time_range_new() {
        let start = Utc::now() - chrono::Duration::days(1);
        let end = Utc::now();
        let range = TimeRange::new(start, end);
        assert_eq!(range.start, start);
        assert_eq!(range.end, end);
    }
}
