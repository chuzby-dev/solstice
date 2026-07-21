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

/// Confidence floor/ceiling: even a razor-thin crossover is worth *some*
/// weight (it's still a real signal), and even a huge divergence never
/// claims near-certainty -- this is a simple heuristic, not a calibrated
/// probability.
const MIN_CONFIDENCE: f64 = 0.5;
const MAX_CONFIDENCE: f64 = 0.95;

/// How strongly the crossover's relative gap maps to confidence: a 1%
/// gap between the short and long SMA adds this many confidence points
/// (before clamping to [`MIN_CONFIDENCE`], [`MAX_CONFIDENCE`]). Chosen so
/// a decisive, well-separated crossover (~0.9% gap on an asset like SOL)
/// reaches the confidence ceiling, while a marginal, just-crossed gap
/// sits close to the floor.
const CONFIDENCE_GAP_SCALE: f64 = 50.0;

pub struct SimpleMovingAverageStrategy {
    pair: TokenPair,
    short_period: usize,
    long_period: usize,
    history: Mutex<VecDeque<f64>>,
}

impl SimpleMovingAverageStrategy {
    pub fn new(pair: TokenPair, short_period: usize, long_period: usize) -> Self {
        SimpleMovingAverageStrategy {
            pair,
            short_period,
            long_period,
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

    /// Confidence scaled by how decisively the short SMA has crossed the
    /// long SMA -- a signal on a wide, well-separated crossover carries
    /// more weight than one on a crossover that just barely happened.
    fn crossover_confidence(short_sma: f64, long_sma: f64) -> f64 {
        if long_sma <= 0.0 {
            return MIN_CONFIDENCE;
        }
        let relative_gap = ((short_sma - long_sma) / long_sma).abs();
        (MIN_CONFIDENCE + relative_gap * CONFIDENCE_GAP_SCALE).clamp(MIN_CONFIDENCE, MAX_CONFIDENCE)
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
            let confidence = Self::crossover_confidence(short_sma, long_sma);
            let mut signal = Signal::new(
                self.name().to_string(),
                SignalType::Buy { pair: self.pair },
                confidence,
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

    #[test]
    fn test_crossover_confidence_at_zero_gap_is_floor() {
        let confidence = SimpleMovingAverageStrategy::crossover_confidence(100.0, 100.0);
        assert_eq!(confidence, MIN_CONFIDENCE);
    }

    #[test]
    fn test_crossover_confidence_scales_with_gap() {
        let narrow = SimpleMovingAverageStrategy::crossover_confidence(100.1, 100.0);
        let wide = SimpleMovingAverageStrategy::crossover_confidence(105.0, 100.0);
        assert!(narrow > MIN_CONFIDENCE);
        assert!(wide > narrow);
    }

    #[test]
    fn test_crossover_confidence_clamps_to_ceiling() {
        let confidence = SimpleMovingAverageStrategy::crossover_confidence(1000.0, 100.0);
        assert_eq!(confidence, MAX_CONFIDENCE);
    }

    #[tokio::test]
    async fn test_wider_crossover_yields_higher_confidence_signal() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let strategy = SimpleMovingAverageStrategy::new(pair, 2, 4);
        let portfolio = PortfolioState::empty();

        // A sharp jump on the last tick should produce a wider short/long
        // gap (and thus higher confidence) than the earlier, gentler climb.
        let prices = [100.0, 100.5, 101.0, 101.5, 150.0];
        let mut signals_by_tick = Vec::new();
        for price in prices {
            signals_by_tick.push(
                strategy
                    .evaluate(&snapshot_with_price(pair, price), &portfolio, &json!({}))
                    .await
                    .unwrap(),
            );
        }

        let early_confidence = signals_by_tick[3]
            .first()
            .expect("should have a signal once enough history accumulates")
            .confidence;
        let late_confidence = signals_by_tick[4]
            .first()
            .expect("should have a signal on the sharp jump")
            .confidence;
        assert!(late_confidence > early_confidence);
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
