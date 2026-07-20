//! Simple moving average crossover strategy (reference implementation).
//!
//! A [`MarketSnapshot`] is a single point in time, so unlike the spec's
//! sketch (which assumes the snapshot itself somehow carries history),
//! this strategy maintains its own rolling price window internally,
//! updated on every `evaluate` call.

use crate::error::StrategyResult;
use crate::strategy::Strategy;
use crate::types::{MarketSnapshot, PortfolioState, StrategyMetadata};
use async_trait::async_trait;
use serde_json::json;
use solstice_core::types::{Signal, SignalType, TokenPair};
use std::collections::VecDeque;
use tokio::sync::Mutex;

pub struct SimpleMovingAverageStrategy {
    pair: TokenPair,
    short_period: usize,
    long_period: usize,
    confidence: f64,
    history: Mutex<VecDeque<f64>>,
}

impl SimpleMovingAverageStrategy {
    pub fn new(pair: TokenPair, short_period: usize, long_period: usize) -> Self {
        SimpleMovingAverageStrategy {
            pair,
            short_period,
            long_period,
            confidence: 0.65,
            history: Mutex::new(VecDeque::new()),
        }
    }

    fn sma(history: &VecDeque<f64>, period: usize) -> Option<f64> {
        if history.len() < period {
            return None;
        }
        let sum: f64 = history.iter().rev().take(period).sum();
        Some(sum / period as f64)
    }
}

#[async_trait]
impl Strategy for SimpleMovingAverageStrategy {
    fn name(&self) -> &str {
        "SMA"
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
        let Some(price) = snapshot.best_price(&self.pair) else {
            return Ok(Vec::new());
        };

        let (short_sma, long_sma) = {
            let mut history = self.history.lock().await;
            history.push_back(price.value);
            // Keep only what the longer window needs.
            while history.len() > self.long_period {
                history.pop_front();
            }
            (
                Self::sma(&history, self.short_period),
                Self::sma(&history, self.long_period),
            )
        };

        let (Some(short_sma), Some(long_sma)) = (short_sma, long_sma) else {
            return Ok(Vec::new());
        };

        if short_sma > long_sma {
            let mut signal = Signal::new(
                self.name().to_string(),
                SignalType::Buy { pair: self.pair },
                self.confidence,
            );
            signal.metadata = json!({ "short_sma": short_sma, "long_sma": long_sma });
            Ok(vec![signal])
        } else {
            Ok(Vec::new())
        }
    }

    fn metadata(&self) -> StrategyMetadata {
        StrategyMetadata {
            name: self.name().to_string(),
            version: self.version().to_string(),
            author: "Solstice".to_string(),
            description: "Simple moving average crossover".to_string(),
            capabilities: vec!["spot_trading".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::Price;

    fn snapshot_with_price(pair: TokenPair, value: f64) -> MarketSnapshot {
        let mut snapshot = MarketSnapshot::new(1);
        snapshot
            .prices
            .insert(pair, vec![Price::new(value, pair, 0.9)]);
        snapshot
    }

    #[tokio::test]
    async fn test_no_signal_before_enough_history() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let strategy = SimpleMovingAverageStrategy::new(pair, 2, 4);
        let portfolio = PortfolioState::empty();

        let signals = strategy
            .evaluate(&snapshot_with_price(pair, 100.0), &portfolio, &json!({}))
            .await
            .unwrap();
        assert!(signals.is_empty());
    }

    #[tokio::test]
    async fn test_buy_signal_on_uptrend() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let strategy = SimpleMovingAverageStrategy::new(pair, 2, 4);
        let portfolio = PortfolioState::empty();

        // Feed an uptrend: short-period average should pull above the
        // long-period average once enough history accumulates.
        let prices = [100.0, 101.0, 102.0, 110.0, 120.0];
        let mut last_signals = Vec::new();
        for price in prices {
            last_signals = strategy
                .evaluate(&snapshot_with_price(pair, price), &portfolio, &json!({}))
                .await
                .unwrap();
        }

        assert!(!last_signals.is_empty());
        assert!(matches!(
            last_signals[0].signal_type,
            SignalType::Buy { .. }
        ));
    }

    #[tokio::test]
    async fn test_no_signal_without_price() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let strategy = SimpleMovingAverageStrategy::new(pair, 2, 4);
        let portfolio = PortfolioState::empty();

        let signals = strategy
            .evaluate(&MarketSnapshot::new(1), &portfolio, &json!({}))
            .await
            .unwrap();
        assert!(signals.is_empty());
    }
}
