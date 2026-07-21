//! Parameter optimization (Phase 6.4): run the same historical replay
//! against several candidate strategy configurations and rank the results.
//!
//! Strategies in this workspace (e.g. `SimpleMovingAverageStrategy`) take
//! their tunable parameters as constructor arguments, not through the
//! `serde_json::Value` blob `StrategyConfig::strategy_config` threads to
//! `Strategy::evaluate` (every current strategy implementation ignores
//! that argument). So rather than sweeping JSON config values that
//! wouldn't actually change strategy behavior, this framework sweeps
//! caller-constructed strategy *instances* — each candidate is a fully
//! built `Vec<Arc<dyn Strategy>>`, e.g. several `SimpleMovingAverageStrategy`
//! instances with different window sizes.

use super::data::HistoricalTick;
use super::engine::{BacktestConfig, BacktestEngine};
use super::report::{BacktestReport, PerformanceMetrics};
use crate::error::SimulationResult;
use solstice_strategy::{Strategy, StrategyConfig, StrategyManager};
use std::sync::Arc;

/// One point in a parameter sweep: a human-readable label and the fully
/// constructed strategies to run the replay against.
pub struct ParameterCandidate {
    pub label: String,
    pub strategies: Vec<Arc<dyn Strategy>>,
}

/// The outcome of running one candidate's backtest.
pub struct SweepResult {
    pub label: String,
    pub report: BacktestReport,
}

/// Run `ticks` through every candidate's strategies (each candidate gets
/// its own freshly built [`StrategyManager`], so no state — e.g. an SMA's
/// rolling price history — leaks between runs), then sort the results by
/// `rank_by` descending (higher is better — e.g. pass
/// `|m| m.total_return_pct` or `|m| m.sharpe_ratio.unwrap_or(f64::MIN)`).
pub async fn optimize_grid(
    candidates: Vec<ParameterCandidate>,
    ticks: &[HistoricalTick],
    strategy_config: StrategyConfig,
    backtest_config: BacktestConfig,
    rank_by: impl Fn(&PerformanceMetrics) -> f64,
) -> SimulationResult<Vec<SweepResult>> {
    let mut results = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let manager = StrategyManager::new(strategy_config.clone());
        for strategy in candidate.strategies {
            // A strategy failing to register (duplicate name, bad config)
            // is a candidate-construction bug the caller should see, not
            // something to silently skip mid-sweep.
            manager.register_strategy(strategy).await?;
        }

        let engine = BacktestEngine::new(Arc::new(manager), backtest_config);
        let report = engine.run(ticks).await?;
        results.push(SweepResult {
            label: candidate.label,
            report,
        });
    }

    results.sort_by(|a, b| {
        rank_by(&b.report.metrics)
            .partial_cmp(&rank_by(&a.report.metrics))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(results)
}

/// Cartesian product of two parameter lists — a small helper for building
/// two-dimensional grids (e.g. SMA short/long window candidates) without
/// hand-writing nested loops at every call site.
pub fn cartesian_product<A: Clone, B: Clone>(a: &[A], b: &[B]) -> Vec<(A, B)> {
    a.iter()
        .flat_map(|x| b.iter().map(move |y| (x.clone(), y.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::data::HistoricalTick;
    use crate::backtest::engine::BacktestConfig;
    use crate::backtest::fill_model::FillModel;
    use chrono::{Duration, Utc};
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::TokenPair;
    use solstice_execution::risk::{
        ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits,
        RiskLimits,
    };
    use solstice_strategy::strategies::sma::SimpleMovingAverageStrategy;

    #[test]
    fn test_cartesian_product() {
        let a = vec![1, 2];
        let b = vec!["x", "y"];
        let product = cartesian_product(&a, &b);
        assert_eq!(product, vec![(1, "x"), (1, "y"), (2, "x"), (2, "y")]);
    }

    fn test_risk_limits() -> RiskLimits {
        RiskLimits {
            position: PositionLimits {
                max_single_position_usd: 50_000,
                max_position_percent: 0.5,
                min_position_size_usd: 10,
                max_open_positions: 10,
            },
            daily_loss: DailyLossLimits {
                max_daily_loss_usd: 1_000_000,
                max_daily_loss_percent: 1.0,
            },
            exposure: ExposureLimits {
                max_total_exposure_usd: 1_000_000,
                max_leverage: 10.0,
            },
            concentration: ConcentrationLimits {
                max_single_asset_percent: 1.0,
            },
            order: OrderLimits {
                max_order_size_usd: 50_000,
                max_slippage_percent: 0.5,
            },
        }
    }

    #[tokio::test]
    async fn test_optimize_grid_ranks_candidates_by_return() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let start = Utc::now();
        // A clean, steady uptrend: a short/long window pair that reacts
        // fast (2/3) should catch more of it than a slow one (2/10).
        let ticks: Vec<HistoricalTick> = (0..30)
            .map(|i| HistoricalTick {
                pair,
                price: 100.0 + i as f64 * 2.0,
                timestamp: start + Duration::minutes(i as i64),
            })
            .collect();

        let backtest_config = BacktestConfig {
            initial_capital_usd: 10_000.0,
            risk_limits: test_risk_limits(),
            kelly_fraction: 0.5,
            default_win_loss_ratio: 2.0,
            stop_loss_percent: 0.9,
            fill_model: FillModel::ideal(),
        };

        let candidates = vec![
            ParameterCandidate {
                label: "fast(2,3)".to_string(),
                strategies: vec![Arc::new(SimpleMovingAverageStrategy::new(pair, 2, 3))],
            },
            ParameterCandidate {
                label: "slow(2,10)".to_string(),
                strategies: vec![Arc::new(SimpleMovingAverageStrategy::new(pair, 2, 10))],
            },
        ];

        let results = optimize_grid(
            candidates,
            &ticks,
            StrategyConfig::default(),
            backtest_config,
            |m| m.total_return_pct,
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 2);
        // Sorted descending by return.
        assert!(
            results[0].report.metrics.total_return_pct
                >= results[1].report.metrics.total_return_pct
        );
        let labels: Vec<&str> = results.iter().map(|r| r.label.as_str()).collect();
        assert!(labels.contains(&"fast(2,3)"));
        assert!(labels.contains(&"slow(2,10)"));
    }
}
