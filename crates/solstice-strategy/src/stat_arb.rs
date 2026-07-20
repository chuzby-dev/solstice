//! Statistical arbitrage: correlation analysis and mean-reversion
//! opportunity detection.
//!
//! `docs/STAT_ARBS.md` doesn't exist yet; this is built from
//! `WORKSPACE.md`'s `solstice-strategy` summary ("Identify mispricing
//! opportunities / Correlation analysis / Mean reversion detection").
//! Cointegration detection (also listed there) is deliberately not
//! implemented: a correct cointegration test (e.g. Engle-Granger) needs
//! an ADF unit-root test, which is easy to get subtly wrong without a
//! statistics crate to verify against — flagged as a follow-up rather
//! than attempted from memory, consistent with how DEX integrations with
//! no vetted reference were handled.
//!
//! A [`MarketSnapshot`] is a single point in time, so — like
//! [`crate::strategies::sma::SimpleMovingAverageStrategy`] — this engine
//! keeps its own rolling price history per pair, fed by
//! [`StatArbEngine::observe`].

use solstice_core::types::TokenPair;
use std::collections::HashMap;
use std::sync::RwLock;

const DEFAULT_MAX_WINDOW: usize = 200;

/// A detected statistical arbitrage opportunity.
#[derive(Debug, Clone)]
pub struct Opportunity {
    pub pair: TokenPair,
    pub kind: OpportunityKind,
    /// Strength of the signal (absolute z-score for mean reversion,
    /// correlation coefficient for pairs). Higher is stronger.
    pub score: f64,
}

#[derive(Debug, Clone)]
pub enum OpportunityKind {
    /// Current price has deviated `z_score` standard deviations from its
    /// rolling mean. Positive `z_score` means overpriced (expect it to
    /// fall back toward the mean); negative means underpriced.
    MeanReversion { z_score: f64 },
    /// This pair's returns are highly correlated with `other`'s, offering
    /// a pairs-trading setup if they diverge.
    Correlated { other: TokenPair, correlation: f64 },
}

/// Accumulates price history per pair and detects mean-reversion and
/// correlation-based opportunities.
pub struct StatArbEngine {
    max_window: usize,
    mean_reversion_z_threshold: f64,
    min_correlation: f64,
    history: RwLock<HashMap<TokenPair, Vec<f64>>>,
}

impl StatArbEngine {
    pub fn new(mean_reversion_z_threshold: f64, min_correlation: f64) -> Self {
        StatArbEngine {
            max_window: DEFAULT_MAX_WINDOW,
            mean_reversion_z_threshold,
            min_correlation,
            history: RwLock::new(HashMap::new()),
        }
    }

    /// Record a new price observation for a pair.
    pub fn observe(&self, pair: TokenPair, price: f64) {
        if let Ok(mut history) = self.history.write() {
            let series = history.entry(pair).or_default();
            series.push(price);
            let overflow = series.len().saturating_sub(self.max_window);
            if overflow > 0 {
                series.drain(0..overflow);
            }
        }
    }

    fn mean_and_stddev(series: &[f64]) -> Option<(f64, f64)> {
        let n = series.len();
        if n < 2 {
            return None;
        }
        let mean = series.iter().sum::<f64>() / n as f64;
        let variance = series.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
        Some((mean, variance.sqrt()))
    }

    fn pearson_correlation(a: &[f64], b: &[f64]) -> Option<f64> {
        let n = a.len().min(b.len());
        if n < 2 {
            return None;
        }
        let a = &a[a.len() - n..];
        let b = &b[b.len() - n..];

        let mean_a = a.iter().sum::<f64>() / n as f64;
        let mean_b = b.iter().sum::<f64>() / n as f64;

        let mut cov = 0.0;
        let mut var_a = 0.0;
        let mut var_b = 0.0;
        for i in 0..n {
            let da = a[i] - mean_a;
            let db = b[i] - mean_b;
            cov += da * db;
            var_a += da * da;
            var_b += db * db;
        }

        if var_a <= 0.0 || var_b <= 0.0 {
            return None;
        }
        Some(cov / (var_a.sqrt() * var_b.sqrt()))
    }

    /// Mean-reversion opportunities across all observed pairs with enough
    /// history.
    pub fn mean_reversion_opportunities(&self) -> Vec<Opportunity> {
        let Ok(history) = self.history.read() else {
            return Vec::new();
        };

        let mut opportunities = Vec::new();
        for (pair, series) in history.iter() {
            let Some((mean, stddev)) = Self::mean_and_stddev(series) else {
                continue;
            };
            if stddev <= 0.0 {
                continue;
            }
            let current = *series
                .last()
                .expect("series checked non-empty by mean_and_stddev");
            let z_score = (current - mean) / stddev;

            if z_score.abs() >= self.mean_reversion_z_threshold {
                opportunities.push(Opportunity {
                    pair: *pair,
                    kind: OpportunityKind::MeanReversion { z_score },
                    score: z_score.abs(),
                });
            }
        }
        opportunities
    }

    /// Pairs of observed tokens whose price series are correlated above
    /// the configured threshold.
    pub fn correlated_pairs(&self) -> Vec<Opportunity> {
        let Ok(history) = self.history.read() else {
            return Vec::new();
        };

        let entries: Vec<_> = history.iter().collect();
        let mut opportunities = Vec::new();

        for i in 0..entries.len() {
            for j in (i + 1)..entries.len() {
                let (pair_a, series_a) = entries[i];
                let (pair_b, series_b) = entries[j];

                let Some(correlation) = Self::pearson_correlation(series_a, series_b) else {
                    continue;
                };
                if correlation.abs() >= self.min_correlation {
                    opportunities.push(Opportunity {
                        pair: *pair_a,
                        kind: OpportunityKind::Correlated {
                            other: *pair_b,
                            correlation,
                        },
                        score: correlation.abs(),
                    });
                }
            }
        }
        opportunities
    }

    /// All detected opportunities (mean-reversion + correlation),
    /// combined.
    pub fn find_opportunities(&self) -> Vec<Opportunity> {
        let mut opportunities = self.mean_reversion_opportunities();
        opportunities.extend(self.correlated_pairs());
        opportunities
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
    fn test_no_opportunities_without_enough_history() {
        let engine = StatArbEngine::new(2.0, 0.8);
        let pair = pair();
        engine.observe(pair, 100.0);

        assert!(engine.mean_reversion_opportunities().is_empty());
    }

    #[test]
    fn test_mean_reversion_detected_on_outlier() {
        let engine = StatArbEngine::new(2.0, 0.8);
        let pair = pair();
        for price in [100.0, 100.5, 99.5, 100.2, 99.8, 100.1] {
            engine.observe(pair, price);
        }
        // Extreme outlier relative to the tight cluster above.
        engine.observe(pair, 150.0);

        let opportunities = engine.mean_reversion_opportunities();
        assert_eq!(opportunities.len(), 1);
        assert_eq!(opportunities[0].pair, pair);
        assert!(matches!(
            opportunities[0].kind,
            OpportunityKind::MeanReversion { z_score } if z_score > 0.0
        ));
    }

    #[test]
    fn test_correlated_pairs_detected() {
        let engine = StatArbEngine::new(2.0, 0.9);
        let pair_a = pair();
        let pair_b = pair();

        // Perfectly correlated (b = 2*a).
        for price in [100.0, 101.0, 99.0, 102.0, 98.0] {
            engine.observe(pair_a, price);
            engine.observe(pair_b, price * 2.0);
        }

        let correlated = engine.correlated_pairs();
        assert_eq!(correlated.len(), 1);
        assert!(matches!(
            correlated[0].kind,
            OpportunityKind::Correlated { correlation, .. } if correlation > 0.99
        ));
    }

    #[test]
    fn test_uncorrelated_pairs_not_flagged() {
        let engine = StatArbEngine::new(2.0, 0.9);
        let pair_a = pair();
        let pair_b = pair();

        for price in [100.0, 101.0, 99.0, 102.0, 98.0] {
            engine.observe(pair_a, price);
        }
        for price in [10.0, 80.0, 20.0, 70.0, 30.0] {
            engine.observe(pair_b, price);
        }

        let correlated = engine.correlated_pairs();
        assert!(correlated.is_empty());
    }

    #[test]
    fn test_window_caps_history_length() {
        let engine = StatArbEngine::new(2.0, 0.8);
        let pair = pair();
        for i in 0..(DEFAULT_MAX_WINDOW + 50) {
            engine.observe(pair, i as f64);
        }

        let history = engine.history.read().unwrap();
        assert_eq!(history.get(&pair).unwrap().len(), DEFAULT_MAX_WINDOW);
    }
}
