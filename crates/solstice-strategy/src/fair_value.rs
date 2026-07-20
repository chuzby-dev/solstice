//! Fair value computation: blending multiple price observations into a
//! single intrinsic price estimate.

use chrono::{DateTime, Utc};
use solstice_core::types::{Price, TokenPair};
use std::time::Duration;

/// Computes a confidence- and recency-weighted fair value from multiple
/// price observations of the same pair (e.g. one per DEX/oracle).
#[derive(Debug, Clone, Copy)]
pub struct FairValueEngine {
    /// Time for a price observation's recency weight to halve. Smaller
    /// values make the engine trust recent observations much more than
    /// older ones; larger values weight all recent-ish observations
    /// roughly equally.
    time_decay_half_life: Duration,
}

impl FairValueEngine {
    pub fn new(time_decay_half_life: Duration) -> Self {
        FairValueEngine {
            time_decay_half_life,
        }
    }

    fn recency_weight(&self, timestamp: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
        let age_secs = now.signed_duration_since(timestamp).num_milliseconds() as f64 / 1000.0;
        let half_life_secs = self.time_decay_half_life.as_secs_f64();
        if half_life_secs <= 0.0 {
            return 1.0;
        }
        0.5f64.powf(age_secs.max(0.0) / half_life_secs)
    }

    /// Compute the fair value for `pair` from `prices`. Returns `None` if
    /// `prices` is empty or every observation has zero effective weight
    /// (e.g. all have zero confidence).
    ///
    /// The result's `confidence` is the weighted-average confidence of the
    /// inputs, not inflated by having multiple sources — combining several
    /// low-confidence observations should not itself produce a
    /// high-confidence fair value.
    pub fn compute_fair_value(&self, pair: TokenPair, prices: &[Price]) -> Option<Price> {
        if prices.is_empty() {
            return None;
        }

        let now = Utc::now();
        let mut weighted_value_sum = 0.0;
        let mut weighted_confidence_sum = 0.0;
        let mut weight_sum = 0.0;

        for price in prices {
            let weight = price.confidence.max(0.0) * self.recency_weight(price.timestamp, now);
            weighted_value_sum += price.value * weight;
            weighted_confidence_sum += price.confidence * weight;
            weight_sum += weight;
        }

        if weight_sum <= 0.0 {
            return None;
        }

        Some(Price::new(
            weighted_value_sum / weight_sum,
            pair,
            weighted_confidence_sum / weight_sum,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    fn pair() -> TokenPair {
        TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique())
    }

    #[test]
    fn test_empty_prices_returns_none() {
        let engine = FairValueEngine::new(Duration::from_secs(60));
        assert!(engine.compute_fair_value(pair(), &[]).is_none());
    }

    #[test]
    fn test_single_price_returns_that_price() {
        let engine = FairValueEngine::new(Duration::from_secs(60));
        let pair = pair();
        let price = Price::new(100.0, pair, 0.9);

        let fair_value = engine.compute_fair_value(pair, &[price]).unwrap();
        assert!((fair_value.value - 100.0).abs() < 1e-9);
        assert!((fair_value.confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn test_higher_confidence_source_weighted_more() {
        let engine = FairValueEngine::new(Duration::from_secs(60));
        let pair = pair();
        let low = Price::new(90.0, pair, 0.1);
        let high = Price::new(110.0, pair, 0.9);

        let fair_value = engine.compute_fair_value(pair, &[low, high]).unwrap();
        // Should be pulled much closer to the high-confidence observation.
        assert!(fair_value.value > 100.0);
    }

    #[test]
    fn test_zero_confidence_prices_ignored() {
        let engine = FairValueEngine::new(Duration::from_secs(60));
        let pair = pair();
        let zero = Price::new(1_000_000.0, pair, 0.0);

        assert!(engine.compute_fair_value(pair, &[zero]).is_none());
    }

    #[test]
    fn test_older_observation_weighted_less() {
        let engine = FairValueEngine::new(Duration::from_secs(60));
        let pair = pair();

        let mut old = Price::new(90.0, pair, 0.9);
        old.timestamp = Utc::now() - chrono::Duration::seconds(600); // 10 half-lives ago
        let fresh = Price::new(110.0, pair, 0.9);

        let fair_value = engine.compute_fair_value(pair, &[old, fresh]).unwrap();
        // The decayed-to-near-zero old observation should barely move the
        // result away from the fresh one.
        assert!(fair_value.value > 109.0);
    }
}
