//! Cross-source spread arbitrage strategy (reference implementation).
//!
//! The spec's sketch compares prices of *different* pairs against each
//! other, which doesn't detect arbitrage (that requires comparing several
//! price observations of the *same* pair from different sources). This
//! implementation does the latter, using [`MarketSnapshot::prices`]'
//! per-pair `Vec<Price>` (one entry per source/DEX).

use crate::error::StrategyResult;
use crate::strategy::Strategy;
use crate::types::{MarketSnapshot, PortfolioState, StrategyMetadata};
use async_trait::async_trait;
use serde_json::json;
use solstice_core::types::{Signal, SignalType};

pub struct SpreadArbitrageStrategy {
    min_spread: f64,
    confidence: f64,
}

impl SpreadArbitrageStrategy {
    /// `min_spread_bps`: minimum spread between the cheapest and most
    /// expensive observed price for a pair, in basis points, to trigger a
    /// signal.
    pub fn new(min_spread_bps: u32) -> Self {
        SpreadArbitrageStrategy {
            min_spread: min_spread_bps as f64 / 10_000.0,
            confidence: 0.8,
        }
    }
}

#[async_trait]
impl Strategy for SpreadArbitrageStrategy {
    fn name(&self) -> &str {
        "SpreadArb"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }

    fn validate_config(&self, _config: &serde_json::Value) -> StrategyResult<()> {
        Ok(())
    }

    async fn evaluate(
        &self,
        snapshot: &MarketSnapshot,
        _portfolio: &PortfolioState,
        _config: &serde_json::Value,
    ) -> StrategyResult<Vec<Signal>> {
        let mut signals = Vec::new();

        for (pair, prices) in &snapshot.prices {
            if prices.len() < 2 {
                continue;
            }

            let min = prices.iter().min_by(|a, b| {
                a.value
                    .partial_cmp(&b.value)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let max = prices.iter().max_by(|a, b| {
                a.value
                    .partial_cmp(&b.value)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let (Some(min), Some(max)) = (min, max) else {
                continue;
            };
            if min.value <= 0.0 {
                continue;
            }

            let spread = (max.value - min.value) / min.value;
            if spread <= self.min_spread {
                continue;
            }

            let mut signal = Signal::new(
                self.name().to_string(),
                SignalType::Buy { pair: *pair },
                self.confidence,
            );
            signal.metadata = json!({
                "spread": spread,
                "low_price": min.value,
                "high_price": max.value,
            });
            signals.push(signal);
        }

        Ok(signals)
    }

    fn metadata(&self) -> StrategyMetadata {
        StrategyMetadata {
            name: self.name().to_string(),
            version: self.version().to_string(),
            author: "Solstice".to_string(),
            description: "Cross-source spread arbitrage detection".to_string(),
            capabilities: vec!["spot_trading".to_string(), "arbitrage".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::{Price, TokenPair};

    #[tokio::test]
    async fn test_no_signal_with_single_source() {
        let strategy = SpreadArbitrageStrategy::new(20);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let mut snapshot = MarketSnapshot::new(1);
        snapshot
            .prices
            .insert(pair, vec![Price::new(100.0, pair, 0.9)]);

        let signals = strategy
            .evaluate(&snapshot, &PortfolioState::empty(), &json!({}))
            .await
            .unwrap();
        assert!(signals.is_empty());
    }

    #[tokio::test]
    async fn test_signal_on_wide_spread() {
        let strategy = SpreadArbitrageStrategy::new(20); // 0.2%
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let mut snapshot = MarketSnapshot::new(1);
        snapshot.prices.insert(
            pair,
            vec![Price::new(100.0, pair, 0.9), Price::new(103.0, pair, 0.9)],
        );

        let signals = strategy
            .evaluate(&snapshot, &PortfolioState::empty(), &json!({}))
            .await
            .unwrap();
        assert_eq!(signals.len(), 1);
        assert!(matches!(signals[0].signal_type, SignalType::Buy { .. }));
    }

    #[tokio::test]
    async fn test_no_signal_when_spread_below_threshold() {
        let strategy = SpreadArbitrageStrategy::new(500); // 5%
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let mut snapshot = MarketSnapshot::new(1);
        snapshot.prices.insert(
            pair,
            vec![Price::new(100.0, pair, 0.9), Price::new(100.5, pair, 0.9)],
        );

        let signals = strategy
            .evaluate(&snapshot, &PortfolioState::empty(), &json!({}))
            .await
            .unwrap();
        assert!(signals.is_empty());
    }
}
