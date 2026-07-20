//! Portfolio management: concentration limits and rebalancing signals.

use crate::types::PortfolioState;
use solstice_core::types::{Signal, SignalType, TokenPair};
use std::collections::HashMap;

/// Tracks concentration limits and produces rebalancing signals when a
/// portfolio drifts outside them.
#[derive(Debug, Clone, Copy)]
pub struct PortfolioManager {
    /// Maximum fraction of total portfolio value allowed in a single pair
    /// (e.g. `0.25` = 25%).
    max_concentration: f64,
}

impl PortfolioManager {
    pub fn new(max_concentration: f64) -> Self {
        PortfolioManager { max_concentration }
    }

    /// Fraction of total portfolio value held in each pair. Positions
    /// sharing a pair (e.g. multiple partial fills) are summed together.
    pub fn concentration(&self, portfolio: &PortfolioState) -> HashMap<TokenPair, f64> {
        let mut concentration = HashMap::new();
        if portfolio.total_value_usd <= 0.0 {
            return concentration;
        }

        for position in &portfolio.positions {
            let value = position.quantity.unsigned_abs() as f64 * position.current_price;
            *concentration.entry(position.pair).or_insert(0.0) += value / portfolio.total_value_usd;
        }
        concentration
    }

    /// Pairs currently exceeding [`max_concentration`](Self::max_concentration),
    /// with their actual concentration.
    pub fn over_concentrated(&self, portfolio: &PortfolioState) -> Vec<(TokenPair, f64)> {
        self.concentration(portfolio)
            .into_iter()
            .filter(|(_, pct)| *pct > self.max_concentration)
            .collect()
    }

    /// Rebalance signals for every over-concentrated pair.
    pub fn rebalance_signals(&self, portfolio: &PortfolioState) -> Vec<Signal> {
        self.over_concentrated(portfolio)
            .into_iter()
            .map(|(pair, pct)| {
                Signal::new(
                    "PortfolioManager".to_string(),
                    SignalType::Rebalance {
                        reason: format!(
                            "{pair} at {:.1}% of portfolio, exceeds {:.1}% limit",
                            pct * 100.0,
                            self.max_concentration * 100.0
                        ),
                    },
                    1.0,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::Position;

    fn portfolio_with_position(
        pair: TokenPair,
        quantity: i64,
        price: f64,
        total_value: f64,
    ) -> PortfolioState {
        let mut position = Position::new(pair, quantity, price);
        position.current_price = price;

        PortfolioState {
            timestamp: Utc::now(),
            positions: vec![position],
            total_value_usd: total_value,
            available_capital: 0,
            risk_metrics: crate::types::RiskMetrics::default(),
        }
    }

    #[test]
    fn test_concentration_empty_portfolio() {
        let manager = PortfolioManager::new(0.25);
        let portfolio = PortfolioState::empty();
        assert!(manager.concentration(&portfolio).is_empty());
    }

    #[test]
    fn test_concentration_computed_correctly() {
        let manager = PortfolioManager::new(0.25);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        // 100 units at $2 = $200 position, against $1000 total -> 20%.
        let portfolio = portfolio_with_position(pair, 100, 2.0, 1000.0);

        let concentration = manager.concentration(&portfolio);
        assert!((concentration[&pair] - 0.2).abs() < 1e-9);
    }

    #[test]
    fn test_over_concentrated_flagged() {
        let manager = PortfolioManager::new(0.25);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        // 100 units at $5 = $500 position, against $1000 total -> 50%.
        let portfolio = portfolio_with_position(pair, 100, 5.0, 1000.0);

        let over = manager.over_concentrated(&portfolio);
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].0, pair);
    }

    #[test]
    fn test_under_limit_not_flagged() {
        let manager = PortfolioManager::new(0.25);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let portfolio = portfolio_with_position(pair, 100, 1.0, 1000.0);

        assert!(manager.over_concentrated(&portfolio).is_empty());
    }

    #[test]
    fn test_rebalance_signal_generated_for_over_concentration() {
        let manager = PortfolioManager::new(0.25);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let portfolio = portfolio_with_position(pair, 100, 5.0, 1000.0);

        let signals = manager.rebalance_signals(&portfolio);
        assert_eq!(signals.len(), 1);
        assert!(matches!(
            signals[0].signal_type,
            SignalType::Rebalance { .. }
        ));
    }
}
