//! Paper trading engine: polls live on-chain quotes, runs the strategy
//! framework against them, sizes and risk-checks resulting signals, and
//! simulates fills — no real transactions are ever built or submitted.

use crate::error::{SimulationError, SimulationResult};
use chrono::{DateTime, Utc};
use serde::Serialize;
use solana_sdk::pubkey::Pubkey;
use solstice_core::types::{Position, PositionId, Signal, SignalType, TokenPair};
use solstice_dex::{DexClient, QuoteRequest};
use solstice_execution::risk::RiskLimits;
use solstice_execution::{
    signal_pair, ExecutionPlan, Fill, OrderManager, PositionSizer, PreTradeRiskChecker,
    RiskMonitor, RiskParams, StopLossManager, TradeApproval,
};
use solstice_strategy::{
    FairValueEngine, MarketSnapshot, PortfolioState, StatArbEngine, StrategyManager,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Real-time events emitted during each tick, for subscribers such as a
/// WebSocket server. Broadcasting is best-effort: if there are no
/// subscribers, or a subscriber is too slow and misses messages, the
/// engine keeps running unaffected — this channel never blocks trading.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum EngineEvent {
    PriceUpdate {
        pair_label: String,
        dex: String,
        price: f64,
        timestamp: DateTime<Utc>,
    },
    SignalGenerated {
        strategy: String,
        pair_label: String,
        confidence: f64,
    },
    OrderFilled {
        order_id: String,
        strategy: String,
        pair_label: String,
        size_usd: u64,
        price: f64,
    },
    TickCompleted {
        timestamp: DateTime<Utc>,
        signal_count: usize,
    },
}

/// JSON-friendly snapshot of a single position.
#[derive(Debug, Clone, Serialize)]
pub struct PositionSnapshot {
    pub pair_label: String,
    pub base_mint: String,
    pub quote_mint: String,
    pub quantity: i64,
    pub entry_price: f64,
    pub current_price: f64,
    pub unrealized_pnl: f64,
}

/// JSON-friendly snapshot of the whole simulated portfolio.
#[derive(Debug, Clone, Serialize)]
pub struct PortfolioSnapshot {
    pub cash_usd: f64,
    pub realized_pnl_usd: f64,
    pub unrealized_pnl_usd: f64,
    pub total_value_usd: f64,
    pub positions: Vec<PositionSnapshot>,
}

/// A pair to monitor, and the pool(s) to quote it against.
#[derive(Debug, Clone)]
pub struct MonitoredPair {
    pub pair: TokenPair,
    pub label: &'static str,
    pub raydium_pool: Option<Pubkey>,
    pub orca_pool: Option<Pubkey>,
    /// Amount of the base token (in its raw base units) to request a quote
    /// for when sampling price, e.g. `1_000_000_000` for 1 SOL (9 decimals).
    pub reference_amount: u64,
}

pub struct PaperTradingConfig {
    pub poll_interval: Duration,
    pub initial_capital_usd: f64,
    pub risk_limits: RiskLimits,
    pub kelly_fraction: f64,
    pub default_win_loss_ratio: f64,
    pub stop_loss_percent: f64,
}

struct SimPortfolio {
    positions: HashMap<TokenPair, Position>,
    cash_usd: f64,
    realized_pnl_usd: f64,
}

/// Live paper-trading loop. Owns no private keys and submits nothing
/// on-chain — every "fill" is simulated using the quote's own execution
/// price.
pub struct PaperTradingEngine {
    raydium: Arc<solstice_dex::RaydiumClient>,
    orca: Arc<solstice_dex::OrcaClient>,
    strategy_manager: Arc<StrategyManager>,
    stat_arb: Arc<StatArbEngine>,
    fair_value: FairValueEngine,
    order_manager: Arc<OrderManager>,
    risk_checker: PreTradeRiskChecker,
    risk_monitor: RiskMonitor,
    stop_loss: StopLossManager,
    portfolio: Mutex<SimPortfolio>,
    pairs: Vec<MonitoredPair>,
    config: PaperTradingConfig,
    events: broadcast::Sender<EngineEvent>,
}

impl PaperTradingEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        raydium: Arc<solstice_dex::RaydiumClient>,
        orca: Arc<solstice_dex::OrcaClient>,
        strategy_manager: Arc<StrategyManager>,
        pairs: Vec<MonitoredPair>,
        config: PaperTradingConfig,
    ) -> Self {
        for p in &pairs {
            if let Some(pool) = p.raydium_pool {
                raydium.register_pool(p.pair.base, p.pair.quote, pool);
            }
            if let Some(pool) = p.orca_pool {
                orca.register_pool(p.pair.base, p.pair.quote, pool);
            }
        }

        let risk_limits = config.risk_limits;
        let (events, _) = broadcast::channel(1024);
        PaperTradingEngine {
            raydium,
            orca,
            strategy_manager,
            stat_arb: Arc::new(StatArbEngine::new(2.0, 0.85)),
            fair_value: FairValueEngine::new(Duration::from_secs(30)),
            order_manager: Arc::new(OrderManager::new()),
            risk_checker: PreTradeRiskChecker::new(risk_limits),
            risk_monitor: RiskMonitor::new(risk_limits),
            stop_loss: StopLossManager::new(config.stop_loss_percent),
            portfolio: Mutex::new(SimPortfolio {
                positions: HashMap::new(),
                cash_usd: config.initial_capital_usd,
                realized_pnl_usd: 0.0,
            }),
            pairs,
            config,
            events,
        }
    }

    /// Subscribe to real-time engine events (price updates, signals,
    /// fills). Each subscriber gets its own receiver; a slow or absent
    /// subscriber never affects the engine.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    /// The order manager, for callers (e.g. an API server) that want to
    /// query order/fill history directly.
    pub fn order_manager(&self) -> &Arc<OrderManager> {
        &self.order_manager
    }

    /// Labels of every pair this engine is configured to monitor.
    pub fn pair_labels(&self) -> Vec<String> {
        self.pairs.iter().map(|p| p.label.to_string()).collect()
    }

    /// Whether the daily-loss circuit breaker has tripped (manual reset
    /// only — see [`RiskMonitor`]).
    pub fn circuit_breaker_tripped(&self) -> bool {
        self.risk_monitor.is_circuit_breaker_tripped()
    }

    fn pair_label(&self, pair: &TokenPair) -> String {
        self.pairs
            .iter()
            .find(|p| p.pair == *pair)
            .map(|p| p.label.to_string())
            .unwrap_or_else(|| pair.to_string())
    }

    /// JSON-friendly snapshot of the current simulated portfolio.
    pub fn portfolio_snapshot(&self) -> PortfolioSnapshot {
        let portfolio = self.portfolio.lock().expect("portfolio lock poisoned");
        let positions: Vec<PositionSnapshot> = portfolio
            .positions
            .values()
            .map(|p| PositionSnapshot {
                pair_label: self.pair_label(&p.pair),
                base_mint: p.pair.base.to_string(),
                quote_mint: p.pair.quote.to_string(),
                quantity: p.quantity,
                entry_price: p.entry_price,
                current_price: p.current_price,
                unrealized_pnl: p.unrealized_pnl(),
            })
            .collect();

        let unrealized_pnl_usd: f64 = positions.iter().map(|p| p.unrealized_pnl).sum();
        let position_value: f64 = portfolio
            .positions
            .values()
            .map(|p| p.quantity as f64 * p.current_price)
            .sum();

        PortfolioSnapshot {
            cash_usd: portfolio.cash_usd,
            realized_pnl_usd: portfolio.realized_pnl_usd,
            unrealized_pnl_usd,
            total_value_usd: portfolio.cash_usd + position_value,
            positions,
        }
    }

    /// Run forever, polling and evaluating on `config.poll_interval`.
    pub async fn run(&self) {
        let mut interval = tokio::time::interval(self.config.poll_interval);
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                warn!("Paper trading tick failed: {}", e);
            }
        }
    }

    /// One evaluation cycle: sample prices, evaluate strategies, act on
    /// resulting signals. Returns the signals produced (post validation/
    /// dedup/ranking) for callers that want to inspect what happened.
    pub async fn tick(&self) -> SimulationResult<Vec<Signal>> {
        let snapshot = self.sample_market().await?;
        let portfolio_state = self.portfolio_state();

        self.evaluate_stop_losses(&portfolio_state);

        let signals = self
            .strategy_manager
            .evaluate_all(&snapshot, &portfolio_state)
            .await;
        info!("Evaluated strategies: {} signal(s)", signals.len());

        for signal in &signals {
            if let Some(pair) = signal_pair(signal) {
                self.emit(EngineEvent::SignalGenerated {
                    strategy: signal.strategy.clone(),
                    pair_label: self.pair_label(&pair),
                    confidence: signal.confidence,
                });
            }
            if let Err(e) = self.act_on_signal(signal, &snapshot).await {
                warn!("Failed to act on signal from {}: {}", signal.strategy, e);
            }
        }

        let metrics = self.risk_monitor.update(
            portfolio_state.positions.len(),
            portfolio_state.total_value_usd as u64,
            0,
            portfolio_state.total_value_usd as u64,
        );
        if self.risk_monitor.is_circuit_breaker_tripped() {
            warn!("Circuit breaker tripped: {:?}", metrics.limits_status);
        }

        self.emit(EngineEvent::TickCompleted {
            timestamp: Utc::now(),
            signal_count: signals.len(),
        });

        Ok(signals)
    }

    /// Broadcast an event to any subscribers. Never fails the caller: a
    /// channel with no subscribers yields `Err`, which is expected and
    /// silently ignored.
    fn emit(&self, event: EngineEvent) {
        let _ = self.events.send(event);
    }

    async fn sample_market(&self) -> SimulationResult<MarketSnapshot> {
        let mut snapshot = MarketSnapshot::new(0);

        for monitored in &self.pairs {
            let mut observations = Vec::new();

            if monitored.raydium_pool.is_some() {
                match self.quote_price(self.raydium.as_ref(), monitored).await {
                    Ok(price) => {
                        info!("[{}] Raydium: ${:.4}", monitored.label, price);
                        self.emit(EngineEvent::PriceUpdate {
                            pair_label: monitored.label.to_string(),
                            dex: "Raydium".to_string(),
                            price,
                            timestamp: Utc::now(),
                        });
                        observations.push(solstice_core::types::Price::new(
                            price,
                            monitored.pair,
                            0.9,
                        ));
                    }
                    Err(e) => warn!("[{}] Raydium quote failed: {}", monitored.label, e),
                }
            }

            if monitored.orca_pool.is_some() {
                match self.quote_price(self.orca.as_ref(), monitored).await {
                    Ok(price) => {
                        info!("[{}] Orca: ${:.4}", monitored.label, price);
                        self.emit(EngineEvent::PriceUpdate {
                            pair_label: monitored.label.to_string(),
                            dex: "Orca".to_string(),
                            price,
                            timestamp: Utc::now(),
                        });
                        observations.push(solstice_core::types::Price::new(
                            price,
                            monitored.pair,
                            0.9,
                        ));
                    }
                    Err(e) => warn!("[{}] Orca quote failed: {}", monitored.label, e),
                }
            }

            if !observations.is_empty() {
                if let Some(fair) = self
                    .fair_value
                    .compute_fair_value(monitored.pair, &observations)
                {
                    self.stat_arb.observe(monitored.pair, fair.value);
                }
                snapshot.prices.insert(monitored.pair, observations);
            }
        }

        Ok(snapshot)
    }

    async fn quote_price(
        &self,
        client: &dyn DexClient,
        monitored: &MonitoredPair,
    ) -> SimulationResult<f64> {
        let request = QuoteRequest::new(
            monitored.pair.base,
            monitored.pair.quote,
            monitored.reference_amount,
            50,
        );
        let quote = client.get_quote(&request).await?;
        if quote.in_amount == 0 {
            return Err(SimulationError::NoPrice);
        }
        // USDC has 6 decimals; base reference_amount is denominated in the
        // base mint's own smallest units, so this ratio is directly a
        // "quote units per base unit" price for a USDC-quoted pair.
        Ok(quote.out_amount as f64 / 1_000_000.0 / (monitored.reference_amount as f64 / 1e9))
    }

    fn portfolio_state(&self) -> PortfolioState {
        let portfolio = self.portfolio.lock().expect("portfolio lock poisoned");
        let position_value: f64 = portfolio
            .positions
            .values()
            .map(|p| p.quantity as f64 * p.current_price)
            .sum();

        PortfolioState {
            timestamp: Utc::now(),
            positions: portfolio.positions.values().cloned().collect(),
            total_value_usd: portfolio.cash_usd + position_value,
            available_capital: portfolio.cash_usd.max(0.0) as u64,
            risk_metrics: solstice_strategy::RiskMetrics::default(),
        }
    }

    fn evaluate_stop_losses(&self, portfolio_state: &PortfolioState) {
        for trigger in self.stop_loss.evaluate_stops(&portfolio_state.positions) {
            warn!(
                "Stop loss triggered for position {:?}: {}",
                trigger.position_id, trigger.reason
            );
            self.close_position_by_id(trigger.position_id);
        }
    }

    fn close_position_by_id(&self, id: PositionId) {
        let mut portfolio = self.portfolio.lock().expect("portfolio lock poisoned");
        if let Some((pair, position)) = portfolio
            .positions
            .iter()
            .find(|(_, p)| p.id == id)
            .map(|(pair, p)| (*pair, p.clone()))
        {
            let pnl = position.unrealized_pnl();
            portfolio.cash_usd += position.quantity as f64 * position.current_price;
            portfolio.realized_pnl_usd += pnl;
            portfolio.positions.remove(&pair);
            info!(
                "Closed position {} for pair {:?}, realized P&L ${:.2}",
                id.0, pair, pnl
            );
        }
    }

    async fn act_on_signal(
        &self,
        signal: &Signal,
        snapshot: &MarketSnapshot,
    ) -> SimulationResult<()> {
        let Some(pair) = signal_pair(signal) else {
            return Ok(());
        };
        let Some(price) = snapshot.best_price(&pair) else {
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

        // Headroom left in this pair before the single-position cap is
        // reached, so an existing position isn't ignored and topped up
        // past the limit on every tick.
        let remaining_position_headroom = (self.config.risk_limits.position.max_single_position_usd
            as f64
            - existing_position_usd)
            .max(0.0);
        if remaining_position_headroom <= 0.0 {
            info!(
                "Signal from {} skipped: {} already at position cap",
                signal.strategy, pair
            );
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
            Err(e) => {
                info!("Signal from {} sized to zero: {}", signal.strategy, e);
                return Ok(());
            }
        };

        let approval = self.risk_checker.check_before_trade(
            size_usd,
            portfolio_state.total_value_usd as u64,
            portfolio_state.positions.len(),
            total_exposure_usd as u64,
            0,
            Some(0.005),
        );

        if !matches!(approval, TradeApproval::Approved) {
            info!("Signal from {} rejected: {:?}", signal.strategy, approval);
            return Ok(());
        }

        let quote = solstice_dex::Quote {
            in_amount: size_usd,
            out_amount: (size_usd as f64 / price.value) as u64,
            fee_amount: 0,
            fee_bps: 0,
            price_impact: 0.0,
            liquidity: 0,
            route: vec![],
            timestamp: Utc::now(),
        };

        let plan = ExecutionPlan {
            signal: signal.clone(),
            pair,
            quote,
            size_usd,
            approval,
        };

        let order_id = self.order_manager.submit(plan)?;
        self.order_manager.record_fill(
            &order_id,
            Fill {
                amount: size_usd,
                price: price.value,
                fee: 0.0,
                timestamp: Utc::now(),
                tx_signature: None,
            },
        )?;

        self.open_or_grow_position(pair, signal, size_usd, price.value);
        info!(
            "SIMULATED FILL: {} {:?} {} for ${} @ ${:.4}",
            signal.strategy, signal.signal_type, order_id, size_usd, price.value
        );
        self.emit(EngineEvent::OrderFilled {
            order_id,
            strategy: signal.strategy.clone(),
            pair_label: self.pair_label(&pair),
            size_usd,
            price: price.value,
        });
        Ok(())
    }

    fn open_or_grow_position(&self, pair: TokenPair, signal: &Signal, size_usd: u64, price: f64) {
        if price <= 0.0 {
            return;
        }
        let quantity_delta = (size_usd as f64 / price).round() as i64;
        let signed_delta = match signal.signal_type {
            SignalType::Sell { .. } => -quantity_delta,
            _ => quantity_delta,
        };

        let mut portfolio = self.portfolio.lock().expect("portfolio lock poisoned");
        portfolio.cash_usd -= size_usd as f64;

        portfolio
            .positions
            .entry(pair)
            .and_modify(|p| {
                p.quantity += signed_delta;
                p.current_price = price;
            })
            .or_insert_with(|| {
                let mut position = Position::new(pair, signed_delta, price);
                position.current_price = price;
                position
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solstice_execution::risk::{
        ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits,
    };
    use solstice_strategy::StrategyConfig;
    use std::str::FromStr;

    fn test_risk_limits() -> RiskLimits {
        RiskLimits {
            position: PositionLimits {
                max_single_position_usd: 5_000,
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

    /// No pools are registered, so `sample_market`/`tick` would never make
    /// a network call -- but these tests drive `act_on_signal` and the
    /// stop-loss/position helpers directly, which never touch the network
    /// regardless.
    fn test_engine(stop_loss_percent: f64) -> PaperTradingEngine {
        let rpc = Arc::new(
            solstice_blockchain::SolanaRpcClient::with_endpoints(vec![
                "http://127.0.0.1:1".to_string()
            ])
            .unwrap(),
        );
        let raydium = Arc::new(solstice_dex::RaydiumClient::new(rpc.clone()));
        let orca = Arc::new(solstice_dex::OrcaClient::new(rpc));
        let strategy_manager = Arc::new(StrategyManager::new(StrategyConfig::default()));
        let pair = test_pair();

        let monitored = MonitoredPair {
            pair,
            label: "TEST/USDC",
            raydium_pool: None,
            orca_pool: None,
            reference_amount: 1_000_000_000,
        };

        let config = PaperTradingConfig {
            poll_interval: Duration::from_secs(3600),
            initial_capital_usd: 10_000.0,
            risk_limits: test_risk_limits(),
            kelly_fraction: 0.5,
            default_win_loss_ratio: 2.0,
            stop_loss_percent,
        };

        PaperTradingEngine::new(raydium, orca, strategy_manager, vec![monitored], config)
    }

    fn test_pair() -> TokenPair {
        // Fixed, not random: `test_engine`'s `MonitoredPair` and each
        // test's snapshot/signal need to agree on the same pair.
        TokenPair::new(
            Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
            Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap(),
        )
    }

    fn snapshot_with_price(pair: TokenPair, price: f64) -> MarketSnapshot {
        let mut snapshot = MarketSnapshot::new(0);
        snapshot.prices.insert(
            pair,
            vec![solstice_core::types::Price::new(price, pair, 0.9)],
        );
        snapshot
    }

    fn buy_signal(pair: TokenPair, confidence: f64) -> Signal {
        Signal::new("Test".to_string(), SignalType::Buy { pair }, confidence)
    }

    #[tokio::test]
    async fn test_act_on_signal_opens_a_position_and_debits_cash() {
        let engine = test_engine(0.1);
        let pair = test_pair();
        let snapshot = snapshot_with_price(pair, 100.0);
        let signal = buy_signal(pair, 0.9);

        engine.act_on_signal(&signal, &snapshot).await.unwrap();

        let snap = engine.portfolio_snapshot();
        assert_eq!(snap.positions.len(), 1);
        assert!(snap.positions[0].quantity > 0);
        assert_eq!(snap.positions[0].entry_price, 100.0);
        assert!(
            snap.cash_usd < 10_000.0,
            "cash should be debited by the fill"
        );
        assert_eq!(engine.order_manager().all_orders().len(), 1);
        assert_eq!(
            engine.order_manager().all_orders()[0].status,
            solstice_execution::OrderStatus::Filled
        );
    }

    #[tokio::test]
    async fn test_act_on_signal_no_price_is_a_noop() {
        let engine = test_engine(0.1);
        let pair = test_pair();
        let empty_snapshot = MarketSnapshot::new(0);
        let signal = buy_signal(pair, 0.9);

        engine
            .act_on_signal(&signal, &empty_snapshot)
            .await
            .unwrap();

        assert!(engine.portfolio_snapshot().positions.is_empty());
        assert!(engine.order_manager().all_orders().is_empty());
    }

    #[tokio::test]
    async fn test_act_on_signal_respects_position_cap() {
        // max_single_position_usd is 5,000; a first fill near the cap
        // should leave no headroom for a second one on the same pair.
        let engine = test_engine(0.1);
        let pair = test_pair();
        let snapshot = snapshot_with_price(pair, 100.0);

        engine
            .act_on_signal(&buy_signal(pair, 0.99), &snapshot)
            .await
            .unwrap();
        let after_first = engine.portfolio_snapshot();
        let first_size = after_first.positions[0].quantity as f64 * 100.0;
        assert!(
            first_size > 4_000.0,
            "first fill should be sized near the cap"
        );

        engine
            .act_on_signal(&buy_signal(pair, 0.99), &snapshot)
            .await
            .unwrap();
        let after_second = engine.portfolio_snapshot();

        // Still exactly one position, and it didn't grow past the cap.
        assert_eq!(after_second.positions.len(), 1);
        let second_size = after_second.positions[0].quantity as f64 * 100.0;
        assert!(second_size <= 5_000.0);
    }

    #[tokio::test]
    async fn test_stop_loss_closes_losing_position() {
        let engine = test_engine(0.05); // 5% stop loss
        let pair = test_pair();
        let snapshot = snapshot_with_price(pair, 100.0);

        engine
            .act_on_signal(&buy_signal(pair, 0.9), &snapshot)
            .await
            .unwrap();
        assert_eq!(engine.portfolio_snapshot().positions.len(), 1);

        // Crash the price in the position directly (mirrors what a real
        // tick would do via `sample_market`, without needing live quotes).
        {
            let mut portfolio = engine.portfolio.lock().unwrap();
            for position in portfolio.positions.values_mut() {
                position.current_price = 80.0; // -20%, past the 5% stop
            }
        }

        let state = engine.portfolio_state();
        engine.evaluate_stop_losses(&state);

        let snap = engine.portfolio_snapshot();
        assert!(
            snap.positions.is_empty(),
            "losing position should have closed"
        );
        assert!(snap.realized_pnl_usd < 0.0);
    }

    #[tokio::test]
    async fn test_portfolio_snapshot_computes_total_value() {
        let engine = test_engine(0.1);
        let pair = test_pair();
        let snapshot = snapshot_with_price(pair, 100.0);

        engine
            .act_on_signal(&buy_signal(pair, 0.9), &snapshot)
            .await
            .unwrap();

        let snap = engine.portfolio_snapshot();
        let expected_position_value = snap.positions[0].quantity as f64 * 100.0;
        assert!((snap.total_value_usd - (snap.cash_usd + expected_position_value)).abs() < 1e-6);
        // No price movement yet, so no unrealized P&L.
        assert_eq!(snap.unrealized_pnl_usd, 0.0);
    }

    #[test]
    fn test_pair_labels_and_circuit_breaker_on_fresh_engine() {
        let engine = test_engine(0.1);
        assert_eq!(engine.pair_labels(), vec!["TEST/USDC".to_string()]);
        assert!(!engine.circuit_breaker_tripped());
    }
}
