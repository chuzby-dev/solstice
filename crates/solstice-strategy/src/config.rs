//! Strategy framework configuration.

use std::time::Duration;

/// Default parameters strategies may fall back to when their own
/// per-strategy config doesn't specify a value.
#[derive(Debug, Clone)]
pub struct StrategyDefaults {
    pub min_confidence: f64,
    pub max_slippage_percent: f64,
    pub min_spread_basis_points: u32,
    pub position_decay_hours: u32,
}

impl Default for StrategyDefaults {
    fn default() -> Self {
        StrategyDefaults {
            min_confidence: 0.65,
            max_slippage_percent: 1.5,
            min_spread_basis_points: 5,
            position_decay_hours: 24,
        }
    }
}

/// Signal deduplication settings.
#[derive(Debug, Clone)]
pub struct DeduplicationConfig {
    pub enabled: bool,
    pub ttl: Duration,
}

impl Default for DeduplicationConfig {
    fn default() -> Self {
        DeduplicationConfig {
            enabled: true,
            ttl: Duration::from_secs(60),
        }
    }
}

/// Top-level strategy framework configuration.
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub max_concurrent_strategies: usize,
    pub evaluation_interval: Duration,
    pub signal_batch_size: usize,
    pub defaults: StrategyDefaults,
    pub deduplication: DeduplicationConfig,
    /// Per-strategy configuration blobs, keyed by strategy name. Passed
    /// to `Strategy::evaluate` as-is; each strategy interprets its own.
    pub strategy_config: serde_json::Value,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        StrategyConfig {
            max_concurrent_strategies: 5,
            evaluation_interval: Duration::from_millis(100),
            signal_batch_size: 100,
            defaults: StrategyDefaults::default(),
            deduplication: DeduplicationConfig::default(),
            strategy_config: serde_json::json!({}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let config = StrategyConfig::default();
        assert_eq!(config.max_concurrent_strategies, 5);
        assert!(config.deduplication.enabled);
        assert_eq!(config.defaults.min_confidence, 0.65);
    }
}
