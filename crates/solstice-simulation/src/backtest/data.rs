//! Historical price data loading.
//!
//! Nothing in this workspace ingests a specific vendor's historical-data
//! API (Birdeye, CoinGecko, Dune, ...), and `solstice-storage`'s
//! `market_snapshots` table only has data for pairs this platform has
//! itself observed live. A two-column CSV (`timestamp,price`) is the
//! common export shape for all of those sources, so it's the interchange
//! format a backtest reads — export from wherever, feed it in here.

use crate::error::{SimulationError, SimulationResult};
use chrono::{DateTime, Utc};
use solstice_core::types::TokenPair;
use std::path::Path;

/// A single historical price observation to replay through the backtest
/// engine, in place of a live-polled [`solstice_core::types::Price`].
#[derive(Debug, Clone, Copy)]
pub struct HistoricalTick {
    pub pair: TokenPair,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
}

/// Load a historical price series from a CSV file with an `timestamp,price`
/// header (RFC 3339 timestamps, e.g. `2026-01-01T00:00:00Z`), returning
/// ticks sorted ascending by time. Every row is validated (parseable
/// timestamp, finite positive price); the first bad row fails the whole
/// load rather than silently dropping data a backtest would then be run
/// against without the caller knowing rows were skipped.
pub fn load_csv(path: &Path, pair: TokenPair) -> SimulationResult<Vec<HistoricalTick>> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| SimulationError::HistoricalData(format!("{}: {e}", path.display())))?;

    let mut ticks = Vec::new();
    for (row_index, result) in reader.records().enumerate() {
        let record =
            result.map_err(|e| SimulationError::HistoricalData(format!("row {row_index}: {e}")))?;

        if record.len() < 2 {
            return Err(SimulationError::HistoricalData(format!(
                "row {row_index}: expected 2 columns (timestamp,price), got {}",
                record.len()
            )));
        }

        let timestamp: DateTime<Utc> = record[0].trim().parse().map_err(|_| {
            SimulationError::HistoricalData(format!(
                "row {row_index}: invalid RFC3339 timestamp {:?}",
                &record[0]
            ))
        })?;
        let price: f64 = record[1].trim().parse().map_err(|_| {
            SimulationError::HistoricalData(format!(
                "row {row_index}: invalid price {:?}",
                &record[1]
            ))
        })?;
        if !price.is_finite() || price <= 0.0 {
            return Err(SimulationError::HistoricalData(format!(
                "row {row_index}: price must be finite and positive, got {price}"
            )));
        }

        ticks.push(HistoricalTick {
            pair,
            price,
            timestamp,
        });
    }

    if ticks.is_empty() {
        return Err(SimulationError::HistoricalData(format!(
            "{}: no data rows found",
            path.display()
        )));
    }

    ticks.sort_by_key(|t| t.timestamp);
    Ok(ticks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use std::io::Write;

    fn pair() -> TokenPair {
        TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique())
    }

    fn write_csv(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{contents}").unwrap();
        file
    }

    #[test]
    fn test_load_csv_sorts_and_parses() {
        let file = write_csv(
            "timestamp,price\n\
             2026-01-01T00:02:00Z,102.0\n\
             2026-01-01T00:00:00Z,100.0\n\
             2026-01-01T00:01:00Z,101.0\n",
        );
        let ticks = load_csv(file.path(), pair()).unwrap();
        assert_eq!(ticks.len(), 3);
        assert_eq!(ticks[0].price, 100.0);
        assert_eq!(ticks[1].price, 101.0);
        assert_eq!(ticks[2].price, 102.0);
        assert!(ticks[0].timestamp < ticks[1].timestamp);
    }

    #[test]
    fn test_load_csv_rejects_non_positive_price() {
        let file = write_csv("timestamp,price\n2026-01-01T00:00:00Z,-5.0\n");
        let result = load_csv(file.path(), pair());
        assert!(matches!(result, Err(SimulationError::HistoricalData(_))));
    }

    #[test]
    fn test_load_csv_rejects_bad_timestamp() {
        let file = write_csv("timestamp,price\nnot-a-time,100.0\n");
        let result = load_csv(file.path(), pair());
        assert!(matches!(result, Err(SimulationError::HistoricalData(_))));
    }

    #[test]
    fn test_load_csv_rejects_empty_file() {
        let file = write_csv("timestamp,price\n");
        let result = load_csv(file.path(), pair());
        assert!(matches!(result, Err(SimulationError::HistoricalData(_))));
    }
}
