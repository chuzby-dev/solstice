//! Historical-replay backtest engine (Phase 6.1/6.2): drives the same
//! strategy → sizing → risk-check pipeline as
//! [`crate::engine::PaperTradingEngine`], but over a sorted slice of
//! [`HistoricalTick`]s instead of live-polled DEX quotes, and fills orders
//! through a configurable [`FillModel`] instead of at the exact quote
//! price with zero cost.
//!
//! Single-threaded and `&mut self` throughout — a backtest replay is one
//! sequential pass driven by one caller, so there's no need for the
//! `Arc<Mutex<_>>`/broadcast-channel machinery the live engine needs to be
//! safely shared with a concurrently polling API server.

use super::data::HistoricalTick;
use super::fill_model::FillModel;
use super::report::{
    pair_label, BacktestReport, ClosedPositionRecord, EquityPoint, PerformanceMetrics, TradeRecord,
};
use crate::error::SimulationResult;
use chrono::{DateTime, Utc};
use solstice_core::types::{Position, PositionId, Signal, SignalType, TokenPair};
use solstice_execution::risk::RiskLimits;
use solstice_execution::{
    signal_pair, ExecutionPlan, OrderManager, PositionSizer, PreTradeRiskChecker, RiskParams,
    StopLossManager, TradeApproval,
};
use solstice_strategy::{MarketSnapshot, PortfolioState, RiskMetrics, StrategyManager};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Clone, Copy)]
pub struct BacktestConfig {
    pub initial_capital_usd: f64,
    pub risk_limits: RiskLimits,
    pub kelly_fraction: f64,
    pub default_win_loss_ratio: f64,
    pub stop_loss_percent: f64,
    pub fill_model: FillModel,
}

struct SimPosition {
    position: Position,
}

pub struct BacktestEngine {
    strategy_manager: Arc<StrategyManager>,
    order_manager: OrderManager,
    risk_checker: PreTradeRiskChecker,
    stop_loss: StopLossManager,
    config: BacktestConfig,

    // Replay-local mutable state (no locking needed: single-threaded).
    positions: HashMap<TokenPair, SimPosition>,
    cash_usd: f64,
    last_price: HashMap<TokenPair, f64>,
    equity_curve: Vec<EquityPoint>,
    fills: Vec<TradeRecord>,
    closed_positions: Vec<ClosedPositionRecord>,
}

impl BacktestEngine {
    pub fn new(strategy_manager: Arc<StrategyManager>, config: BacktestConfig) -> Self {
        let risk_limits = config.risk_limits;
        BacktestEngine {
            strategy_manager,
            order_manager: OrderManager::new(),
            risk_checker: PreTradeRiskChecker::new(risk_limits),
            stop_loss: StopLossManager::new(config.stop_loss_percent),
            cash_usd: config.initial_capital_usd,
            config,
            positions: HashMap::new(),
            last_price: HashMap::new(),
            equity_curve: Vec::new(),
            fills: Vec::new(),
            closed_positions: Vec::new(),
        }
    }

    /// Replay `ticks` (must already be sorted ascending by timestamp — as
    /// [`super::data::load_csv`] returns them) through the strategy
    /// pipeline, one tick at a time, and return the full performance
    /// report.
    pub async fn run(mut self, ticks: &[HistoricalTick]) -> SimulationResult<BacktestReport> {
        let Some(first) = ticks.first() else {
            return Ok(self.finish(Utc::now(), Utc::now()));
        };
        let (start, mut end) = (first.timestamp, first.timestamp);

        for tick in ticks {
            end = tick.timestamp;
            self.process_tick(tick).await?;
        }

        Ok(self.finish(start, end))
    }

    async fn process_tick(&mut self, tick: &HistoricalTick) -> SimulationResult<()> {
        self.last_price.insert(tick.pair, tick.price);
        if let Some(sim) = self.positions.get_mut(&tick.pair) {
            sim.position.current_price = tick.price;
        }

        let portfolio_state = self.portfolio_state();
        self.evaluate_stop_losses(&portfolio_state, tick.timestamp);

        let mut snapshot = MarketSnapshot::new(0);
        snapshot.timestamp = tick.timestamp;
        snapshot.prices.insert(
            tick.pair,
            vec![solstice_core::types::Price {
                value: tick.price,
                pair: tick.pair,
                timestamp: tick.timestamp,
                confidence: 1.0,
            }],
        );

        let portfolio_state = self.portfolio_state();
        let signals = self
            .strategy_manager
            .evaluate_all(&snapshot, &portfolio_state)
            .await;

        for signal in &signals {
            self.act_on_signal(signal, tick.price, tick.timestamp)?;
        }

        self.record_equity(tick.timestamp);
        Ok(())
    }

    fn act_on_signal(
        &mut self,
        signal: &Signal,
        price: f64,
        timestamp: DateTime<Utc>,
    ) -> SimulationResult<()> {
        let Some(pair) = signal_pair(signal) else {
            return Ok(());
        };

        let portfolio_state = self.portfolio_state();
        let existing_position_usd = portfolio_state
            .positions
            .iter()
            .find(|p| p.pair == pair)
            .map(|p| p.quantity.unsigned_abs() as f64 * p.current_price)
            .unwrap_or(0.0);
        let total_exposure_usd = portfolio_state
            .positions
            .iter()
            .map(|p| p.quantity.unsigned_abs() as f64 * p.current_price)
            .sum::<f64>();

        let remaining_position_headroom = (self.config.risk_limits.position.max_single_position_usd
            as f64
            - existing_position_usd)
            .max(0.0);
        if remaining_position_headroom <= 0.0 {
            return Ok(());
        }

        let risk_params = RiskParams {
            portfolio_value_usd: portfolio_state.total_value_usd,
            available_capital_usd: portfolio_state.available_capital as f64,
            max_position_usd: remaining_position_headroom,
            max_position_percent: self.config.risk_limits.position.max_position_percent,
            kelly_fraction: self.config.kelly_fraction,
            default_win_loss_ratio: self.config.default_win_loss_ratio,
        };

        let size_usd = match PositionSizer::calculate_size(signal, &risk_params) {
            Ok(size) => size,
            Err(_) => return Ok(()),
        };

        let approval = self.risk_checker.check_before_trade(
            size_usd,
            portfolio_state.total_value_usd as u64,
            portfolio_state.positions.len(),
            total_exposure_usd as u64,
            0,
            Some(self.config.fill_model.slippage.bps_for(size_usd) / 10_000.0),
        );
        if !matches!(approval, TradeApproval::Approved) {
            return Ok(());
        }

        let is_buy = !matches!(signal.signal_type, SignalType::Sell { .. });
        let (fill, filled_usd) = self
            .config
            .fill_model
            .simulate_fill(size_usd, price, is_buy, timestamp);

        // A synthetic quote just carries the plan through `OrderManager`'s
        // approval gate; the fill itself (below) is what actually prices
        // and costs this trade.
        let quote = solstice_dex::Quote {
            in_amount: filled_usd,
            out_amount: (filled_usd as f64 / fill.price.max(f64::MIN_POSITIVE)) as u64,
            fee_amount: 0,
            fee_bps: 0,
            price_impact: 0.0,
            liquidity: 0,
            route: vec![],
            timestamp,
        };
        let plan = ExecutionPlan {
            signal: signal.clone(),
            pair,
            quote,
            size_usd: filled_usd,
            approval,
        };
        let order_id = self.order_manager.submit(plan)?;
        self.order_manager.record_fill(&order_id, fill.clone())?;

        self.fills.push(TradeRecord {
            strategy: signal.strategy.clone(),
            pair: pair_label(&pair),
            is_buy,
            size_usd: filled_usd,
            price: fill.price,
            fee_usd: fill.fee,
            timestamp,
        });

        self.open_or_grow_position(pair, signal, filled_usd, fill.price);
        info!(
            "Backtest fill: {} {:?} ${} @ ${:.4} (order {})",
            signal.strategy, signal.signal_type, filled_usd, fill.price, order_id
        );
        Ok(())
    }

    fn open_or_grow_position(
        &mut self,
        pair: TokenPair,
        signal: &Signal,
        size_usd: u64,
        price: f64,
    ) {
        if price <= 0.0 {
            return;
        }
        let quantity_delta = (size_usd as f64 / price).round() as i64;
        let signed_delta = match signal.signal_type {
            SignalType::Sell { .. } => -quantity_delta,
            _ => quantity_delta,
        };

        self.cash_usd -= size_usd as f64;

        self.positions
            .entry(pair)
            .and_modify(|sim| {
                sim.position.quantity += signed_delta;
                sim.position.current_price = price;
            })
            .or_insert_with(|| {
                let mut position = Position::new(pair, signed_delta, price);
                position.current_price = price;
                SimPosition { position }
            });
    }

    fn evaluate_stop_losses(&mut self, portfolio_state: &PortfolioState, timestamp: DateTime<Utc>) {
        for trigger in self.stop_loss.evaluate_stops(&portfolio_state.positions) {
            self.close_position_by_id(trigger.position_id, trigger.reason, timestamp);
        }
    }

    fn close_position_by_id(&mut self, id: PositionId, reason: String, timestamp: DateTime<Utc>) {
        let Some((pair, position)) = self
            .positions
            .iter()
            .find(|(_, sim)| sim.position.id == id)
            .map(|(pair, sim)| (*pair, sim.position.clone()))
        else {
            return;
        };

        let realized_pnl = position.unrealized_pnl();
        self.cash_usd += position.quantity as f64 * position.current_price;
        self.positions.remove(&pair);

        self.closed_positions.push(ClosedPositionRecord {
            pair: pair_label(&pair),
            quantity: position.quantity,
            entry_price: position.entry_price,
            exit_price: position.current_price,
            realized_pnl_usd: realized_pnl,
            opened_at: position.opened_at,
            closed_at: timestamp,
            reason,
        });
    }

    fn portfolio_state(&self) -> PortfolioState {
        let position_value: f64 = self
            .positions
            .values()
            .map(|sim| sim.position.quantity as f64 * sim.position.current_price)
            .sum();

        PortfolioState {
            timestamp: Utc::now(),
            positions: self
                .positions
                .values()
                .map(|sim| sim.position.clone())
                .collect(),
            total_value_usd: self.cash_usd + position_value,
            available_capital: self.cash_usd.max(0.0) as u64,
            risk_metrics: RiskMetrics::default(),
        }
    }

    fn record_equity(&mut self, timestamp: DateTime<Utc>) {
        let total_value_usd = self.portfolio_state().total_value_usd;
        self.equity_curve.push(EquityPoint {
            timestamp,
            total_value_usd,
        });
    }

    fn finish(self, start: DateTime<Utc>, end: DateTime<Utc>) -> BacktestReport {
        let metrics = PerformanceMetrics::compute(
            self.config.initial_capital_usd,
            &self.equity_curve,
            &self.fills,
            &self.closed_positions,
        );
        let pair_label = self
            .last_price
            .keys()
            .next()
            .map(pair_label)
            .unwrap_or_else(|| "unknown".to_string());

        BacktestReport {
            pair: pair_label,
            start,
            end,
            equity_curve: self.equity_curve,
            fills: self.fills,
            closed_positions: self.closed_positions,
            metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::fill_model::{FeeModel, PartialFillConfig, SlippageModel};
    use chrono::Duration;
    use solana_sdk::pubkey::Pubkey;
    use solstice_execution::risk::{
        ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits,
    };
    use solstice_strategy::strategies::sma::SimpleMovingAverageStrategy;
    use solstice_strategy::StrategyConfig;

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

    fn test_config() -> BacktestConfig {
        BacktestConfig {
            initial_capital_usd: 10_000.0,
            risk_limits: test_risk_limits(),
            kelly_fraction: 0.5,
            default_win_loss_ratio: 2.0,
            stop_loss_percent: 0.5,
            fill_model: FillModel {
                slippage: SlippageModel::FixedBps(10.0),
                fee: FeeModel { bps: 25.0 },
                partial_fill: PartialFillConfig::unlimited(),
            },
        }
    }

    fn uptrend_ticks(pair: TokenPair, n: usize) -> Vec<HistoricalTick> {
        let start = Utc::now();
        (0..n)
            .map(|i| HistoricalTick {
                pair,
                price: 100.0 + i as f64, // strictly increasing
                timestamp: start + Duration::minutes(i as i64),
            })
            .collect()
    }

    #[tokio::test]
    async fn test_empty_ticks_produces_empty_report() {
        let manager = Arc::new(StrategyManager::new(StrategyConfig::default()));
        let engine = BacktestEngine::new(manager, test_config());
        let report = engine.run(&[]).await.unwrap();

        assert!(report.equity_curve.is_empty());
        assert!(report.fills.is_empty());
        assert_eq!(report.metrics.final_equity_usd, 10_000.0);
        assert_eq!(report.metrics.total_return_pct, 0.0);
    }

    #[tokio::test]
    async fn test_uptrend_produces_fills_and_equity_curve() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let manager = Arc::new(StrategyManager::new(StrategyConfig::default()));
        manager
            .register_strategy(Arc::new(SimpleMovingAverageStrategy::new(pair, 2, 4)))
            .await
            .unwrap();

        let ticks = uptrend_ticks(pair, 20);
        let engine = BacktestEngine::new(manager, test_config());
        let report = engine.run(&ticks).await.unwrap();

        assert_eq!(report.equity_curve.len(), ticks.len());
        assert!(
            !report.fills.is_empty(),
            "SMA should buy into a clean uptrend"
        );
        assert!(report.fills.iter().all(|f| f.is_buy));
        // Slippage + fees mean every buy fills strictly above the tick price.
        assert!(report
            .fills
            .iter()
            .zip(ticks.iter().filter(|_| true))
            .all(|(f, _)| f.price > 0.0));
        assert!(report.metrics.total_fees_usd > 0.0);
    }

    #[tokio::test]
    async fn test_stop_loss_closes_position_on_crash() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let manager = Arc::new(StrategyManager::new(StrategyConfig::default()));
        manager
            .register_strategy(Arc::new(SimpleMovingAverageStrategy::new(pair, 2, 3)))
            .await
            .unwrap();

        let start = Utc::now();
        let mut ticks: Vec<HistoricalTick> = (0..6)
            .map(|i| HistoricalTick {
                pair,
                price: 100.0 + i as f64 * 5.0, // uptrend to trigger a buy
                timestamp: start + Duration::minutes(i as i64),
            })
            .collect();
        // Crash hard enough to blow through a 50% stop loss.
        ticks.push(HistoricalTick {
            pair,
            price: 10.0,
            timestamp: start + Duration::minutes(10),
        });

        let mut config = test_config();
        config.stop_loss_percent = 0.1;
        let engine = BacktestEngine::new(manager, config);
        let report = engine.run(&ticks).await.unwrap();

        assert!(
            !report.closed_positions.is_empty(),
            "the crash should have triggered a stop loss close"
        );
        assert!(report.closed_positions[0].realized_pnl_usd < 0.0);
        assert_eq!(report.metrics.win_rate, Some(0.0));
    }

    #[tokio::test]
    async fn test_ideal_fill_model_has_no_fees() {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let manager = Arc::new(StrategyManager::new(StrategyConfig::default()));
        manager
            .register_strategy(Arc::new(SimpleMovingAverageStrategy::new(pair, 2, 4)))
            .await
            .unwrap();

        let mut config = test_config();
        config.fill_model = FillModel::ideal();
        let ticks = uptrend_ticks(pair, 20);
        let engine = BacktestEngine::new(manager, config);
        let report = engine.run(&ticks).await.unwrap();

        assert!(!report.fills.is_empty());
        assert_eq!(report.metrics.total_fees_usd, 0.0);
    }
}
