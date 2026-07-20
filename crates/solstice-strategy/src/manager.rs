//! Strategy registration and coordinated evaluation.
//!
//! Deviates from `docs/STRATEGY_FRAMEWORK.md`'s sketch in one deliberate
//! way: that spec loads strategies from `.so`/`.dll` files at runtime via
//! `libloading` and an `extern "C" fn create_strategy() -> *mut dyn Strategy`
//! ABI boundary. Rust has no stable ABI across compiler versions, so that
//! pattern is fragile in practice (a strategy compiled with a different
//! rustc than the host will typically produce undefined behavior, not a
//! clean error) and this workspace has no compiled plugin `.so` to
//! validate such loading against. `StrategyManager` instead registers
//! already-constructed `Arc<dyn Strategy>` values — strategies are Rust
//! crates compiled into the host binary (or, for genuine hot-reloading,
//! run out-of-process behind an RPC boundary), which is the pattern most
//! production Rust plugin systems converge on for the same reason. Dynamic
//! `.so` loading can be added later if a real need for it appears.

use crate::config::StrategyConfig;
use crate::deduplicator::SignalDeduplicator;
use crate::error::{StrategyError, StrategyResult};
use crate::ranker::SignalRanker;
use crate::strategy::Strategy;
use crate::types::{MarketSnapshot, PortfolioState};
use crate::validator::SignalValidator;
use solstice_core::types::Signal;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

pub struct StrategyManager {
    strategies: RwLock<HashMap<String, Arc<dyn Strategy>>>,
    config: StrategyConfig,
    validator: SignalValidator,
    deduplicator: SignalDeduplicator,
}

impl StrategyManager {
    pub fn new(config: StrategyConfig) -> Self {
        let validator = SignalValidator::new(config.defaults.min_confidence);
        let deduplicator = SignalDeduplicator::new(config.deduplication.ttl);

        StrategyManager {
            strategies: RwLock::new(HashMap::new()),
            config,
            validator,
            deduplicator,
        }
    }

    /// Validate, initialize, and register a strategy. Rejects a second
    /// registration under the same name, and rejects exceeding
    /// `config.max_concurrent_strategies`.
    pub async fn register_strategy(&self, strategy: Arc<dyn Strategy>) -> StrategyResult<()> {
        strategy.validate_config(&self.config.strategy_config)?;

        let mut strategies = self.strategies.write().await;
        if strategies.contains_key(strategy.name()) {
            return Err(StrategyError::AlreadyRegistered(
                strategy.name().to_string(),
            ));
        }
        if strategies.len() >= self.config.max_concurrent_strategies {
            return Err(StrategyError::InvalidConfig(format!(
                "max_concurrent_strategies ({}) reached",
                self.config.max_concurrent_strategies
            )));
        }

        strategy.initialize().await?;
        info!(
            "Registered strategy: {} v{}",
            strategy.name(),
            strategy.version()
        );
        strategies.insert(strategy.name().to_string(), strategy);
        Ok(())
    }

    pub async fn unregister_strategy(&self, name: &str) -> StrategyResult<()> {
        let mut strategies = self.strategies.write().await;
        let strategy = strategies
            .remove(name)
            .ok_or_else(|| StrategyError::NotFound(name.to_string()))?;
        drop(strategies);

        strategy.shutdown().await?;
        info!("Unregistered strategy: {}", name);
        Ok(())
    }

    pub async fn registered_strategies(&self) -> Vec<String> {
        self.strategies.read().await.keys().cloned().collect()
    }

    /// Evaluate every registered strategy concurrently, validate and
    /// deduplicate the resulting signals, and return them ranked by
    /// confidence (highest first).
    ///
    /// A strategy that panics or returns an error is logged and excluded;
    /// it never aborts evaluation of the others.
    pub async fn evaluate_all(
        &self,
        snapshot: &MarketSnapshot,
        portfolio: &PortfolioState,
    ) -> Vec<Signal> {
        let strategies: Vec<Arc<dyn Strategy>> =
            self.strategies.read().await.values().cloned().collect();

        let mut handles = Vec::with_capacity(strategies.len());
        for strategy in strategies {
            let snapshot = snapshot.clone();
            let portfolio = portfolio.clone();
            let config = self.config.strategy_config.clone();
            handles.push(tokio::spawn(async move {
                let name = strategy.name().to_string();
                let result = strategy.evaluate(&snapshot, &portfolio, &config).await;
                (name, result)
            }));
        }

        let mut all_signals = Vec::new();
        for handle in handles {
            match handle.await {
                Ok((_, Ok(signals))) => all_signals.extend(signals),
                Ok((name, Err(e))) => error!("Strategy {} evaluation failed: {}", name, e),
                Err(e) => error!("Strategy evaluation task panicked: {}", e),
            }
        }

        let valid = self.validator.filter_valid(all_signals);
        let deduplicated = if self.config.deduplication.enabled {
            self.deduplicator.deduplicate(valid).await
        } else {
            valid
        };

        let mut ranked = SignalRanker::rank(deduplicated);
        ranked.truncate(self.config.signal_batch_size);
        ranked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::{Signal, SignalType, TokenPair};

    struct AlwaysSignalsStrategy {
        name: &'static str,
        confidence: f64,
    }

    #[async_trait]
    impl Strategy for AlwaysSignalsStrategy {
        fn name(&self) -> &str {
            self.name
        }

        fn version(&self) -> &str {
            "1.0.0"
        }

        fn validate_config(&self, _config: &serde_json::Value) -> StrategyResult<()> {
            Ok(())
        }

        async fn evaluate(
            &self,
            _snapshot: &MarketSnapshot,
            _portfolio: &PortfolioState,
            _config: &serde_json::Value,
        ) -> StrategyResult<Vec<Signal>> {
            let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
            Ok(vec![Signal::new(
                self.name.to_string(),
                SignalType::Buy { pair },
                self.confidence,
            )])
        }
    }

    struct FailingStrategy;

    #[async_trait]
    impl Strategy for FailingStrategy {
        fn name(&self) -> &str {
            "Failing"
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn validate_config(&self, _config: &serde_json::Value) -> StrategyResult<()> {
            Ok(())
        }
        async fn evaluate(
            &self,
            _snapshot: &MarketSnapshot,
            _portfolio: &PortfolioState,
            _config: &serde_json::Value,
        ) -> StrategyResult<Vec<Signal>> {
            Err(StrategyError::EvaluationFailed("boom".to_string()))
        }
    }

    #[tokio::test]
    async fn test_register_and_list() {
        let manager = StrategyManager::new(StrategyConfig::default());
        manager
            .register_strategy(Arc::new(AlwaysSignalsStrategy {
                name: "A",
                confidence: 0.9,
            }))
            .await
            .unwrap();

        assert_eq!(manager.registered_strategies().await, vec!["A".to_string()]);
    }

    #[tokio::test]
    async fn test_duplicate_registration_rejected() {
        let manager = StrategyManager::new(StrategyConfig::default());
        manager
            .register_strategy(Arc::new(AlwaysSignalsStrategy {
                name: "A",
                confidence: 0.9,
            }))
            .await
            .unwrap();

        let result = manager
            .register_strategy(Arc::new(AlwaysSignalsStrategy {
                name: "A",
                confidence: 0.9,
            }))
            .await;
        assert!(matches!(result, Err(StrategyError::AlreadyRegistered(_))));
    }

    #[tokio::test]
    async fn test_unregister_unknown_fails() {
        let manager = StrategyManager::new(StrategyConfig::default());
        let result = manager.unregister_strategy("Nope").await;
        assert!(matches!(result, Err(StrategyError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_evaluate_all_ranks_by_confidence() {
        let manager = StrategyManager::new(StrategyConfig::default());
        manager
            .register_strategy(Arc::new(AlwaysSignalsStrategy {
                name: "Low",
                confidence: 0.7,
            }))
            .await
            .unwrap();
        manager
            .register_strategy(Arc::new(AlwaysSignalsStrategy {
                name: "High",
                confidence: 0.95,
            }))
            .await
            .unwrap();

        let signals = manager
            .evaluate_all(&MarketSnapshot::new(1), &PortfolioState::empty())
            .await;

        assert_eq!(signals.len(), 2);
        assert_eq!(signals[0].strategy, "High");
        assert_eq!(signals[1].strategy, "Low");
    }

    #[tokio::test]
    async fn test_evaluate_all_excludes_failing_strategy() {
        let manager = StrategyManager::new(StrategyConfig::default());
        manager
            .register_strategy(Arc::new(FailingStrategy))
            .await
            .unwrap();
        manager
            .register_strategy(Arc::new(AlwaysSignalsStrategy {
                name: "Ok",
                confidence: 0.9,
            }))
            .await
            .unwrap();

        let signals = manager
            .evaluate_all(&MarketSnapshot::new(1), &PortfolioState::empty())
            .await;

        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].strategy, "Ok");
    }

    #[tokio::test]
    async fn test_max_concurrent_strategies_enforced() {
        let config = StrategyConfig {
            max_concurrent_strategies: 1,
            ..StrategyConfig::default()
        };
        let manager = StrategyManager::new(config);
        manager
            .register_strategy(Arc::new(AlwaysSignalsStrategy {
                name: "A",
                confidence: 0.9,
            }))
            .await
            .unwrap();

        let result = manager
            .register_strategy(Arc::new(AlwaysSignalsStrategy {
                name: "B",
                confidence: 0.9,
            }))
            .await;
        assert!(matches!(result, Err(StrategyError::InvalidConfig(_))));
    }
}
