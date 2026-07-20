//! The core `Strategy` trait every strategy implements.

use crate::error::StrategyResult;
use crate::types::{MarketSnapshot, PortfolioState, StrategyMetadata};
use async_trait::async_trait;
use solstice_core::types::Signal;

/// A pluggable trading strategy.
///
/// Implementations must be `Send + Sync`: a [`crate::manager::StrategyManager`]
/// holds them behind `Arc<dyn Strategy>` and evaluates all registered
/// strategies concurrently.
#[async_trait]
pub trait Strategy: Send + Sync {
    /// Strategy name. Used as its registry key, so must be unique across
    /// all strategies registered with a given [`crate::manager::StrategyManager`].
    fn name(&self) -> &str;

    /// Strategy version (semver-style, e.g. `"1.0.0"`), for observability
    /// and to distinguish signals from different revisions of the same
    /// strategy over time.
    fn version(&self) -> &str;

    /// Validate a strategy-specific configuration blob before the strategy
    /// is registered or (re-)configured.
    fn validate_config(&self, config: &serde_json::Value) -> StrategyResult<()>;

    /// Evaluate current market/portfolio state and produce zero or more
    /// signals. Called on every evaluation cycle by the manager.
    async fn evaluate(
        &self,
        market_snapshot: &MarketSnapshot,
        portfolio_state: &PortfolioState,
        config: &serde_json::Value,
    ) -> StrategyResult<Vec<Signal>>;

    /// Called once when the strategy is registered, before any `evaluate`
    /// call. Default is a no-op.
    async fn initialize(&self) -> StrategyResult<()> {
        Ok(())
    }

    /// Called once when the strategy is unregistered. Default is a no-op.
    async fn shutdown(&self) -> StrategyResult<()> {
        Ok(())
    }

    /// Descriptive metadata. Default derives a minimal value from
    /// [`name`](Self::name)/[`version`](Self::version); override to add
    /// author/description/capabilities.
    fn metadata(&self) -> StrategyMetadata {
        StrategyMetadata {
            name: self.name().to_string(),
            version: self.version().to_string(),
            author: String::new(),
            description: String::new(),
            capabilities: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubStrategy;

    #[async_trait]
    impl Strategy for StubStrategy {
        fn name(&self) -> &str {
            "Stub"
        }

        fn version(&self) -> &str {
            "0.0.1"
        }

        fn validate_config(&self, _config: &serde_json::Value) -> StrategyResult<()> {
            Ok(())
        }

        async fn evaluate(
            &self,
            _market_snapshot: &MarketSnapshot,
            _portfolio_state: &PortfolioState,
            _config: &serde_json::Value,
        ) -> StrategyResult<Vec<Signal>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn test_default_metadata() {
        let strategy = StubStrategy;
        let metadata = strategy.metadata();
        assert_eq!(metadata.name, "Stub");
        assert_eq!(metadata.version, "0.0.1");
        assert!(metadata.capabilities.is_empty());
    }

    #[tokio::test]
    async fn test_default_initialize_and_shutdown_are_noops() {
        let strategy = StubStrategy;
        assert!(strategy.initialize().await.is_ok());
        assert!(strategy.shutdown().await.is_ok());
    }
}
