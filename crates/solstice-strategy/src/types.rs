//! Data provided to and returned from strategy evaluation.
//!
//! Reuses `solstice-core`'s domain types (`Price`, `OrderBook`, `Position`,
//! `Signal`, `TokenPair`) rather than defining a parallel set, since those
//! are already the canonical types used throughout the platform.

use chrono::{DateTime, Utc};
use solstice_core::types::{OrderBook, Position, Price, TokenPair};
use std::collections::HashMap;

/// Available liquidity for a pool.
#[derive(Debug, Clone, Copy)]
pub struct Liquidity {
    pub reserve_a: u64,
    pub reserve_b: u64,
    /// Fraction of total liquidity currently deployed/in-range, if known
    /// (meaningful for concentrated-liquidity pools; `1.0` otherwise).
    pub utilization: f64,
}

/// Point-in-time view of market state, passed to every strategy on each
/// evaluation cycle.
///
/// `prices` maps a pair to *all* price observations available for it this
/// cycle (one per source/DEX) rather than a single collapsed value, since
/// strategies like spread-arbitrage need to compare sources against each
/// other, not just read a single blended price.
#[derive(Debug, Clone, Default)]
pub struct MarketSnapshot {
    pub timestamp: DateTime<Utc>,
    pub slot: u64,
    pub prices: HashMap<TokenPair, Vec<Price>>,
    pub orderbooks: HashMap<TokenPair, OrderBook>,
    pub liquidity: HashMap<TokenPair, Liquidity>,
    pub volumes_24h: HashMap<TokenPair, u64>,
}

impl MarketSnapshot {
    pub fn new(slot: u64) -> Self {
        MarketSnapshot {
            timestamp: Utc::now(),
            slot,
            prices: HashMap::new(),
            orderbooks: HashMap::new(),
            liquidity: HashMap::new(),
            volumes_24h: HashMap::new(),
        }
    }

    /// The best (highest-confidence) single price for a pair, if any.
    pub fn best_price(&self, pair: &TokenPair) -> Option<&Price> {
        self.prices.get(pair)?.iter().max_by(|a, b| {
            a.confidence
                .partial_cmp(&b.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

/// Portfolio-level risk metrics.
#[derive(Debug, Clone, Default)]
pub struct RiskMetrics {
    pub max_drawdown: f64,
    pub daily_pnl: f64,
    pub daily_loss: f64,
    pub exposure_percent: f64,
    /// Fraction of total portfolio value per pair.
    pub concentration: HashMap<TokenPair, f64>,
}

/// Point-in-time portfolio state, passed to every strategy alongside the
/// market snapshot.
#[derive(Debug, Clone)]
pub struct PortfolioState {
    pub timestamp: DateTime<Utc>,
    pub positions: Vec<Position>,
    pub total_value_usd: f64,
    pub available_capital: u64,
    pub risk_metrics: RiskMetrics,
}

impl PortfolioState {
    pub fn empty() -> Self {
        PortfolioState {
            timestamp: Utc::now(),
            positions: Vec::new(),
            total_value_usd: 0.0,
            available_capital: 0,
            risk_metrics: RiskMetrics::default(),
        }
    }
}

/// Descriptive metadata a strategy reports about itself.
#[derive(Debug, Clone, Default)]
pub struct StrategyMetadata {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub capabilities: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn test_market_snapshot_best_price() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let mut snapshot = MarketSnapshot::new(1);

        let low_confidence = Price::new(100.0, pair, 0.4);
        let high_confidence = Price::new(101.0, pair, 0.9);
        snapshot
            .prices
            .insert(pair, vec![low_confidence, high_confidence]);

        let best = snapshot.best_price(&pair).unwrap();
        assert_eq!(best.value, 101.0);
    }

    #[test]
    fn test_market_snapshot_best_price_missing() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let snapshot = MarketSnapshot::new(1);
        assert!(snapshot.best_price(&pair).is_none());
    }

    #[test]
    fn test_portfolio_state_empty() {
        let state = PortfolioState::empty();
        assert!(state.positions.is_empty());
        assert_eq!(state.total_value_usd, 0.0);
    }
}
