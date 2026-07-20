//! Row types and conversions between `solstice-core` domain types and the
//! Postgres schema.
//!
//! Several core fields are `u64` (token quantities, lamports) while Postgres
//! `BIGINT` is signed 64-bit; conversions go through `TryFrom` and return a
//! [`StorageError::ValueOutOfRange`] rather than silently truncating.

use crate::error::{StorageError, StorageResult};
use chrono::{DateTime, Utc};
use solana_sdk::pubkey::Pubkey;
use solstice_core::types::{Position, PositionId, Trade, TradeAction};
use std::str::FromStr;
use uuid::Uuid;

fn u64_to_i64(column: &'static str, value: u64) -> StorageResult<i64> {
    i64::try_from(value).map_err(|_| StorageError::ValueOutOfRange {
        column,
        value: value.to_string(),
    })
}

fn i64_to_u64(column: &'static str, value: i64) -> StorageResult<u64> {
    u64::try_from(value).map_err(|_| StorageError::ValueOutOfRange {
        column,
        value: value.to_string(),
    })
}

fn parse_pubkey(column: &'static str, value: &str) -> StorageResult<Pubkey> {
    Pubkey::from_str(value).map_err(|_| StorageError::ValueOutOfRange {
        column,
        value: value.to_string(),
    })
}

/// A single price observation, as stored in `market_snapshots`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MarketSnapshotRow {
    pub time: DateTime<Utc>,
    pub base_mint: String,
    pub quote_mint: String,
    pub price: f64,
    pub confidence: f64,
    pub source: String,
}

/// A trade record, as stored in `trades`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TradeRow {
    pub id: String,
    pub position_id: Uuid,
    pub base_mint: String,
    pub quote_mint: String,
    pub action: String,
    pub quantity: i64,
    pub execution_price: f64,
    pub fees: f64,
    pub executed_at: DateTime<Utc>,
    pub tx_signature: Option<String>,
}

impl TradeRow {
    pub fn from_trade(trade: &Trade) -> StorageResult<Self> {
        Ok(TradeRow {
            id: trade.id.clone(),
            position_id: trade.position_id.0,
            base_mint: trade.pair.base.to_string(),
            quote_mint: trade.pair.quote.to_string(),
            action: match trade.action {
                TradeAction::Buy => "buy".to_string(),
                TradeAction::Sell => "sell".to_string(),
            },
            quantity: u64_to_i64("quantity", trade.quantity)?,
            execution_price: trade.execution_price,
            fees: trade.fees,
            executed_at: trade.timestamp,
            tx_signature: trade.tx_signature.clone(),
        })
    }

    pub fn into_trade(self) -> StorageResult<Trade> {
        let base = parse_pubkey("base_mint", &self.base_mint)?;
        let quote = parse_pubkey("quote_mint", &self.quote_mint)?;
        let action = match self.action.as_str() {
            "buy" => TradeAction::Buy,
            "sell" => TradeAction::Sell,
            other => {
                return Err(StorageError::ValueOutOfRange {
                    column: "action",
                    value: other.to_string(),
                })
            }
        };

        Ok(Trade {
            id: self.id,
            position_id: PositionId(self.position_id),
            pair: solstice_core::types::TokenPair::new(base, quote),
            action,
            quantity: i64_to_u64("quantity", self.quantity)?,
            execution_price: self.execution_price,
            fees: self.fees,
            timestamp: self.executed_at,
            tx_signature: self.tx_signature,
        })
    }
}

/// A position state snapshot, as stored in `position_updates`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PositionUpdateRow {
    pub position_id: Uuid,
    pub base_mint: String,
    pub quote_mint: String,
    pub quantity: i64,
    pub entry_price: f64,
    pub current_price: f64,
    pub opened_at: DateTime<Utc>,
    pub close_at: Option<DateTime<Utc>>,
    pub recorded_at: DateTime<Utc>,
}

impl PositionUpdateRow {
    pub fn from_position(position: &Position) -> Self {
        PositionUpdateRow {
            position_id: position.id.0,
            base_mint: position.pair.base.to_string(),
            quote_mint: position.pair.quote.to_string(),
            quantity: position.quantity,
            entry_price: position.entry_price,
            current_price: position.current_price,
            opened_at: position.opened_at,
            close_at: position.close_at,
            recorded_at: Utc::now(),
        }
    }

    pub fn into_position(self) -> StorageResult<Position> {
        let base = parse_pubkey("base_mint", &self.base_mint)?;
        let quote = parse_pubkey("quote_mint", &self.quote_mint)?;

        Ok(Position {
            id: PositionId(self.position_id),
            pair: solstice_core::types::TokenPair::new(base, quote),
            quantity: self.quantity,
            entry_price: self.entry_price,
            current_price: self.current_price,
            opened_at: self.opened_at,
            close_at: self.close_at,
        })
    }
}

/// A raw account state snapshot, as stored in `account_snapshots`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountSnapshotRow {
    pub time: DateTime<Utc>,
    pub address: String,
    pub owner: String,
    pub lamports: i64,
    pub data: Vec<u8>,
    pub slot: i64,
}

impl AccountSnapshotRow {
    pub fn new(
        address: Pubkey,
        owner: Pubkey,
        lamports: u64,
        data: Vec<u8>,
        slot: u64,
        time: DateTime<Utc>,
    ) -> StorageResult<Self> {
        Ok(AccountSnapshotRow {
            time,
            address: address.to_string(),
            owner: owner.to_string(),
            lamports: u64_to_i64("lamports", lamports)?,
            data,
            slot: u64_to_i64("slot", slot)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solstice_core::types::TokenPair;

    #[test]
    fn test_trade_row_roundtrip() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let trade = Trade::new(
            PositionId::new(),
            pair,
            TradeAction::Buy,
            10_000,
            100.0,
            2.5,
        );

        let row = TradeRow::from_trade(&trade).unwrap();
        let roundtripped = row.into_trade().unwrap();

        assert_eq!(roundtripped.id, trade.id);
        assert_eq!(roundtripped.quantity, trade.quantity);
        assert_eq!(roundtripped.pair, trade.pair);
        assert!(matches!(roundtripped.action, TradeAction::Buy));
    }

    #[test]
    fn test_u64_to_i64_out_of_range() {
        let result = u64_to_i64("quantity", u64::MAX);
        assert!(matches!(
            result,
            Err(StorageError::ValueOutOfRange {
                column: "quantity",
                ..
            })
        ));
    }

    #[test]
    fn test_position_update_row_roundtrip() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let position = Position::new(pair, 500, 42.0);

        let row = PositionUpdateRow::from_position(&position);
        let roundtripped = row.into_position().unwrap();

        assert_eq!(roundtripped.id, position.id);
        assert_eq!(roundtripped.quantity, position.quantity);
    }

    #[test]
    fn test_account_snapshot_row_new() {
        let row = AccountSnapshotRow::new(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            1_000_000,
            vec![1, 2, 3],
            42,
            Utc::now(),
        )
        .unwrap();

        assert_eq!(row.lamports, 1_000_000);
        assert_eq!(row.slot, 42);
    }

    #[test]
    fn test_invalid_trade_action_rejected() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let row = TradeRow {
            id: "t1".to_string(),
            position_id: Uuid::new_v4(),
            base_mint: pair.base.to_string(),
            quote_mint: pair.quote.to_string(),
            action: "hold".to_string(),
            quantity: 1,
            execution_price: 1.0,
            fees: 0.0,
            executed_at: Utc::now(),
            tx_signature: None,
        };

        assert!(row.into_trade().is_err());
    }
}
