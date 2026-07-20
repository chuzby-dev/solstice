//! Solstice Strategy Framework
//!
//! Pluggable strategy trait, coordinated concurrent evaluation, and
//! signal validation/deduplication/ranking. See
//! `docs/STRATEGY_FRAMEWORK.md`, and `manager.rs` for one deliberate
//! deviation from it (no dynamic `.so` loading).

pub mod config;
pub mod deduplicator;
pub mod error;
pub mod fair_value;
pub mod manager;
pub mod portfolio;
pub mod ranker;
pub mod stat_arb;
pub mod strategies;
pub mod strategy;
pub mod types;
pub mod validator;

pub use config::{DeduplicationConfig, StrategyConfig, StrategyDefaults};
pub use deduplicator::SignalDeduplicator;
pub use error::{StrategyError, StrategyResult};
pub use fair_value::FairValueEngine;
pub use manager::StrategyManager;
pub use portfolio::PortfolioManager;
pub use ranker::SignalRanker;
pub use stat_arb::{Opportunity, OpportunityKind, StatArbEngine};
pub use strategy::Strategy;
pub use types::{Liquidity, MarketSnapshot, PortfolioState, RiskMetrics, StrategyMetadata};
pub use validator::SignalValidator;

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::StrategyManager>;
    }
}
