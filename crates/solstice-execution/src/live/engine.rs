//! Live trading engine: the same strategy → size → risk-check pipeline as
//! `solstice_simulation::PaperTradingEngine`, but backed by a real wallet
//! and (when armed) real on-chain execution via [`crate::execute_swap`].
//!
//! **Defaults to disabled.** [`LiveTradingEngine::is_enabled`] starts
//! `false` and stays that way until [`LiveTradingEngine::enable`] is
//! called explicitly. While disabled, every tick runs exactly the same
//! signal-generation and position-sizing logic but emits
//! [`LiveEvent::WouldTrade`] instead of calling `execute_swap` — so the
//! dashboard can show "what this would do" before anyone flips it live.
//! [`LiveTradingEngine::disable`] is synchronous and instant, by design:
//! turning trading off should never be blocked on anything.

use super::config::{LiveTradedPair, LiveTradingConfig};
use crate::error::{ExecutionError, ExecutionResult};
use crate::execute_swap;
use crate::jito::{JitoClient, JitoConfig};
use crate::order_manager::{Fill, OrderManager};
use crate::planner::{signal_pair, ExecutionPlan};
use crate::position_sizing::{PositionSizer, RiskParams};
use crate::risk::{PreTradeRiskChecker, StopLossManager, TakeProfitManager, TradeApproval};
use chrono::{DateTime, Utc};
use serde::Serialize;
use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::{SolanaRpcClient, WalletFile};
use solstice_core::types::{Position, PositionId, Signal, SignalType, TokenPair};
use solstice_dex::{
    DexAggregator, JupiterClient, OrcaClient, Quote, QuoteRequest, RaydiumClient, SwapRequest,
};
use solstice_strategy::{MarketSnapshot, PortfolioState, RiskMetrics, StrategyManager};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::warn;

/// Real-time events from the live engine, for the API/dashboard to stream.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum LiveEvent {
    PriceUpdate {
        pair_label: String,
        price: f64,
        timestamp: DateTime<Utc>,
    },
    SignalGenerated {
        strategy: String,
        pair_label: String,
        confidence: f64,
    },
    /// Emitted instead of actually trading, whenever the engine is
    /// disabled. Carries exactly what would have been sized and
    /// risk-checked, so the dashboard can preview live behavior with zero
    /// funds risk.
    WouldTrade {
        strategy: String,
        pair_label: String,
        size_usd: u64,
        is_buy: bool,
    },
    SignalSkipped {
        strategy: String,
        pair_label: String,
        reason: String,
    },
    OrderFilled {
        strategy: String,
        pair_label: String,
        size_usd: u64,
        price: f64,
        method: String,
        signature: Option<String>,
    },
    OrderFailed {
        strategy: String,
        pair_label: String,
        reason: String,
    },
    PositionClosed {
        pair_label: String,
        realized_pnl_usd: f64,
        reason: String,
    },
    LiveTradingEnabled,
    LiveTradingDisabled,
    MaxCapitalChanged {
        max_capital_usd: f64,
    },
    MinConfidenceChanged {
        min_confidence: f64,
    },
    StrategiesEnabledChanged {
        strategies_enabled: bool,
    },
    TakeProfitPercentChanged {
        take_profit_percent: f64,
    },
    CrossDexArbEnabledChanged {
        cross_dex_arb_enabled: bool,
    },
    CrossDexMinSpreadChanged {
        cross_dex_min_spread: f64,
    },
    CrossDexMaxSlippageChanged {
        cross_dex_max_slippage_bps: u32,
    },
    CrossDexMinNetEdgeChanged {
        cross_dex_min_net_edge_bps: u32,
    },
    PairEnabledChanged {
        pair_label: String,
        enabled: bool,
    },
    CrossDexOpportunityDetected {
        pair_label: String,
        buy_dex: String,
        sell_dex: String,
        buy_price: f64,
        sell_price: f64,
        spread_percent: f64,
    },
    CrossDexArbFilled {
        pair_label: String,
        buy_dex: String,
        sell_dex: String,
        size_usd: u64,
        buy_price: f64,
        sell_price: f64,
        realized_pnl_usd: f64,
        buy_signature: Option<String>,
        sell_signature: Option<String>,
    },
    CrossDexArbFailed {
        pair_label: String,
        leg: String,
        reason: String,
    },
    /// An on-chain base-token balance for an arb-configured pair was
    /// found with no matching tracked position -- e.g. a prior sell leg
    /// that failed, or a balance read that raced ahead of a since-fixed
    /// buy confirmation (see the read-consistency note on
    /// `CrossDexArbFailed`'s "buy" leg). Adopted as a tracked position so
    /// it's eligible to be flattened back to quote and protected by
    /// stop-loss/take-profit, rather than sitting invisible forever.
    UntrackedBalanceAdopted {
        pair_label: String,
        quantity: f64,
        estimated_usd: f64,
    },
    TickCompleted {
        timestamp: DateTime<Utc>,
        signal_count: usize,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct PairStatus {
    pub pair_label: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LivePositionSnapshot {
    pub pair_label: String,
    pub quantity_raw: u64,
    pub entry_price: f64,
    pub current_price: f64,
    pub allocated_usd: f64,
    pub unrealized_pnl_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveStatusSnapshot {
    pub enabled: bool,
    pub wallet_address: String,
    pub max_capital_usd: f64,
    pub min_confidence: f64,
    pub strategies_enabled: bool,
    pub take_profit_percent: f64,
    pub cross_dex_arb_enabled: bool,
    pub cross_dex_min_spread: f64,
    pub cross_dex_max_slippage_bps: u32,
    pub cross_dex_min_net_edge_bps: u32,
    pub pairs: Vec<PairStatus>,
    pub capital_deployed_usd: f64,
    pub capital_available_usd: f64,
    pub realized_pnl_usd: f64,
    pub positions: Vec<LivePositionSnapshot>,
}

struct LivePosition {
    id: PositionId,
    pair: TokenPair,
    quantity_raw: u64,
    entry_price: f64,
    current_price: f64,
    allocated_usd: f64,
    opened_at: DateTime<Utc>,
}

/// What a signal, once sized and risk-checked, would do -- computed with
/// no I/O so it's testable without a live network. `None` from
/// `plan_signal` means "skip this signal" (already covered by [`LiveEvent::SignalSkipped`]
/// at the call site).
#[derive(Debug)]
struct PlannedTrade {
    pair: TokenPair,
    pair_label: String,
    is_buy: bool,
    size_usd: u64,
}

/// A detected cross-DEX price discrepancy for a pair: the cheapest
/// registered DEX to buy on and the priciest to sell on, if the gap
/// between them clears `cross_dex_min_spread`. Detecting this is cheap
/// (a handful of quote requests); acting on it is not risk-free -- see
/// `LiveTradingConfig::cross_dex_arb_enabled`'s doc comment.
#[derive(Debug, Clone)]
struct ArbOpportunity {
    buy_dex: String,
    sell_dex: String,
    buy_price: f64,
    sell_price: f64,
    spread: f64,
}

/// The actual spread required to attempt a cross-DEX arb trade: the
/// operator-set `min_spread` floor, or -- if larger -- a cost-aware floor
/// assuming both legs slip against the trade by the full
/// `max_slippage_bps` tolerance plus a required net margin on top. See
/// `LiveTradingConfig::cross_dex_min_net_edge_bps`'s doc comment for why
/// `min_spread` alone isn't a safe gate.
fn required_cross_dex_spread(min_spread: f64, max_slippage_bps: u32, min_net_edge_bps: u32) -> f64 {
    min_spread.max(2.0 * max_slippage_bps as f64 / 10_000.0 + min_net_edge_bps as f64 / 10_000.0)
}

pub struct LiveTradingEngine {
    wallet_pubkey: Pubkey,
    wallet_file: WalletFile,
    rpc: Arc<SolanaRpcClient>,
    jito: JitoClient,
    dex: Arc<DexAggregator>,
    strategy_manager: Arc<StrategyManager>,
    order_manager: Arc<OrderManager>,
    risk_checker: PreTradeRiskChecker,
    stop_loss: StopLossManager,
    pairs: Vec<LiveTradedPair>,
    config: Mutex<LiveTradingConfig>,
    enabled: Arc<AtomicBool>,
    positions: Mutex<HashMap<TokenPair, LivePosition>>,
    capital_deployed_usd: Mutex<f64>,
    realized_pnl_usd: Mutex<f64>,
    events: broadcast::Sender<LiveEvent>,
    /// Jito tip accounts, refreshed at most every [`TIP_ACCOUNTS_TTL`]
    /// rather than on every trade -- fetching them fresh each time adds a
    /// full network round trip between reading a price and submitting the
    /// swap built against it, widening the window for the quote to go
    /// stale and the on-chain route to revert.
    tip_accounts_cache: Mutex<Option<(std::time::Instant, Vec<Pubkey>)>>,
}

/// Tip accounts rotate rarely; a few minutes of staleness costs nothing
/// (any tip account Jito has ever advertised still accepts tips) but saves
/// a network round trip on every trade attempt.
const TIP_ACCOUNTS_TTL: Duration = Duration::from_secs(300);

impl LiveTradingEngine {
    pub fn new(
        wallet_file: WalletFile,
        rpc: Arc<SolanaRpcClient>,
        strategy_manager: Arc<StrategyManager>,
        pairs: Vec<LiveTradedPair>,
        config: LiveTradingConfig,
    ) -> ExecutionResult<Self> {
        let wallet_pubkey = wallet_file
            .pubkey()
            .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;
        let jito = JitoClient::new(JitoConfig::default())
            .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;

        // Jupiter is always registered (it's the only one that needs no
        // pre-known pool address); Orca and Raydium are registered too,
        // with whichever pools each `LiveTradedPair` supplies -- pairs
        // without a known pool for one of those DEXes just never win the
        // best-route comparison for that leg, they're not an error.
        let mut dex = DexAggregator::new();
        dex.register(Arc::new(JupiterClient::new()?));
        let raydium = RaydiumClient::new(rpc.clone());
        let orca = OrcaClient::new(rpc.clone());
        for pair in &pairs {
            if let Some(pool) = pair.raydium_pool {
                raydium.register_pool(pair.base_mint, pair.quote_mint, pool);
            }
            if let Some(pool) = pair.orca_pool {
                orca.register_pool(pair.base_mint, pair.quote_mint, pool);
            }
        }
        dex.register(Arc::new(raydium));
        dex.register(Arc::new(orca));
        let dex = Arc::new(dex);

        let risk_limits = config.risk_limits;
        let (events, _) = broadcast::channel(1024);

        Ok(LiveTradingEngine {
            wallet_pubkey,
            wallet_file,
            rpc,
            jito,
            dex,
            strategy_manager,
            order_manager: Arc::new(OrderManager::new()),
            risk_checker: PreTradeRiskChecker::new(risk_limits),
            stop_loss: StopLossManager::new(config.stop_loss_percent),
            pairs,
            config: Mutex::new(config),
            enabled: Arc::new(AtomicBool::new(false)),
            positions: Mutex::new(HashMap::new()),
            capital_deployed_usd: Mutex::new(0.0),
            realized_pnl_usd: Mutex::new(0.0),
            events,
            tip_accounts_cache: Mutex::new(None),
        })
    }

    /// Return Jito's current tip accounts, using a cached value if it's
    /// fresher than [`TIP_ACCOUNTS_TTL`] and only hitting the network on a
    /// cache miss or expiry.
    async fn cached_tip_accounts(&self) -> Vec<Pubkey> {
        if let Some((fetched_at, accounts)) = self
            .tip_accounts_cache
            .lock()
            .expect("lock poisoned")
            .clone()
        {
            if fetched_at.elapsed() < TIP_ACCOUNTS_TTL {
                return accounts;
            }
        }

        match self.jito.get_tip_accounts().await {
            Ok(accounts) => {
                *self.tip_accounts_cache.lock().expect("lock poisoned") =
                    Some((std::time::Instant::now(), accounts.clone()));
                accounts
            }
            Err(e) => {
                warn!("failed to refresh Jito tip accounts: {}", e);
                Vec::new()
            }
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.events.subscribe()
    }

    pub fn order_manager(&self) -> &Arc<OrderManager> {
        &self.order_manager
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Arm the engine: from the next tick onward, approved signals call
    /// `execute_swap` for real.
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
        self.emit(LiveEvent::LiveTradingEnabled);
    }

    /// Disarm the engine. Synchronous, instant, and always safe to call --
    /// this never itself trades, so there's nothing to wait on.
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.emit(LiveEvent::LiveTradingDisabled);
    }

    pub fn set_max_capital_usd(&self, max_capital_usd: f64) {
        let mut config = self.config.lock().expect("config lock poisoned");
        config.max_capital_usd = max_capital_usd;
        drop(config);
        self.emit(LiveEvent::MaxCapitalChanged { max_capital_usd });
    }

    /// Set the minimum signal confidence required to act on a signal.
    /// `plan_signal` reads this fresh on every signal, so a change takes
    /// effect immediately, not on next restart.
    pub fn set_min_confidence(&self, min_confidence: f64) {
        let mut config = self.config.lock().expect("config lock poisoned");
        config.min_confidence = min_confidence;
        drop(config);
        self.emit(LiveEvent::MinConfidenceChanged { min_confidence });
    }

    /// Enable/disable the strategy-driven signal pipeline
    /// (SMA/SpreadArb), independent of the main kill switch and of
    /// `cross_dex_arb_enabled`. Read fresh from config every `tick`, so a
    /// change takes effect on the next tick.
    pub fn set_strategies_enabled(&self, strategies_enabled: bool) {
        let mut config = self.config.lock().expect("config lock poisoned");
        config.strategies_enabled = strategies_enabled;
        drop(config);
        self.emit(LiveEvent::StrategiesEnabledChanged { strategies_enabled });
    }

    /// Set the fractional gain at which an open position auto-closes.
    /// Read fresh from config on every `evaluate_stop_losses` call, so a
    /// change takes effect on the next tick, not on next restart.
    pub fn set_take_profit_percent(&self, take_profit_percent: f64) {
        let mut config = self.config.lock().expect("config lock poisoned");
        config.take_profit_percent = take_profit_percent;
        drop(config);
        self.emit(LiveEvent::TakeProfitPercentChanged {
            take_profit_percent,
        });
    }

    /// Arms or disarms the cross-DEX arbitrage executor. See
    /// `LiveTradingConfig::cross_dex_arb_enabled`'s doc comment for why
    /// this is a separate gate from `enable`/`disable`: it issues two
    /// non-atomic live transactions per opportunity, a materially
    /// different risk profile from every other trade this engine makes.
    pub fn set_cross_dex_arb_enabled(&self, cross_dex_arb_enabled: bool) {
        let mut config = self.config.lock().expect("config lock poisoned");
        config.cross_dex_arb_enabled = cross_dex_arb_enabled;
        drop(config);
        self.emit(LiveEvent::CrossDexArbEnabledChanged {
            cross_dex_arb_enabled,
        });
    }

    /// Set the minimum spread (e.g. `0.015` = 1.5%) required across
    /// registered DEXes before the cross-DEX arbitrage executor acts.
    /// Read fresh from config every `evaluate_cross_dex_arbitrage` call.
    pub fn set_cross_dex_min_spread(&self, cross_dex_min_spread: f64) {
        let mut config = self.config.lock().expect("config lock poisoned");
        config.cross_dex_min_spread = cross_dex_min_spread;
        drop(config);
        self.emit(LiveEvent::CrossDexMinSpreadChanged {
            cross_dex_min_spread,
        });
    }

    /// Set the per-leg slippage tolerance used only by the cross-DEX
    /// arbitrage executor -- see `LiveTradingConfig::cross_dex_max_slippage_bps`'s
    /// doc comment for why this is kept separate from the general
    /// `slippage_bps` used by ordinary trades.
    pub fn set_cross_dex_max_slippage_bps(&self, cross_dex_max_slippage_bps: u32) {
        let mut config = self.config.lock().expect("config lock poisoned");
        config.cross_dex_max_slippage_bps = cross_dex_max_slippage_bps;
        drop(config);
        self.emit(LiveEvent::CrossDexMaxSlippageChanged {
            cross_dex_max_slippage_bps,
        });
    }

    /// Set the minimum required profit margin (in basis points) that must
    /// remain after assuming both legs of a cross-DEX arb slip against the
    /// trade by the full `cross_dex_max_slippage_bps` tolerance -- see
    /// `LiveTradingConfig::cross_dex_min_net_edge_bps`'s doc comment.
    pub fn set_cross_dex_min_net_edge_bps(&self, cross_dex_min_net_edge_bps: u32) {
        let mut config = self.config.lock().expect("config lock poisoned");
        config.cross_dex_min_net_edge_bps = cross_dex_min_net_edge_bps;
        drop(config);
        self.emit(LiveEvent::CrossDexMinNetEdgeChanged {
            cross_dex_min_net_edge_bps,
        });
    }

    /// Enable or disable a configured pair (by `LiveTradedPair::label`) for
    /// *new* trade consideration -- see `LiveTradingConfig::disabled_pairs`'s
    /// doc comment for why an existing open position keeps being sampled
    /// and closed regardless. No-op (but still emits the event) if
    /// `pair_label` doesn't match any configured pair.
    pub fn set_pair_enabled(&self, pair_label: String, enabled: bool) {
        let mut config = self.config.lock().expect("config lock poisoned");
        if enabled {
            config.disabled_pairs.remove(&pair_label);
        } else {
            config.disabled_pairs.insert(pair_label.clone());
        }
        drop(config);
        self.emit(LiveEvent::PairEnabledChanged {
            pair_label,
            enabled,
        });
    }

    fn pair_label(&self, pair: &TokenPair) -> String {
        self.pairs
            .iter()
            .find(|p| p.base_mint == pair.base && p.quote_mint == pair.quote)
            .map(|p| p.label.to_string())
            .unwrap_or_else(|| pair.to_string())
    }

    pub fn status(&self) -> LiveStatusSnapshot {
        let config = self.config.lock().expect("config lock poisoned");
        let capital_deployed_usd = *self.capital_deployed_usd.lock().expect("lock poisoned");
        let positions = self.positions.lock().expect("positions lock poisoned");

        LiveStatusSnapshot {
            enabled: self.is_enabled(),
            wallet_address: self.wallet_pubkey.to_string(),
            max_capital_usd: config.max_capital_usd,
            min_confidence: config.min_confidence,
            strategies_enabled: config.strategies_enabled,
            take_profit_percent: config.take_profit_percent,
            cross_dex_arb_enabled: config.cross_dex_arb_enabled,
            cross_dex_min_spread: config.cross_dex_min_spread,
            cross_dex_max_slippage_bps: config.cross_dex_max_slippage_bps,
            cross_dex_min_net_edge_bps: config.cross_dex_min_net_edge_bps,
            pairs: self
                .pairs
                .iter()
                .map(|p| PairStatus {
                    pair_label: p.label.to_string(),
                    enabled: !config.disabled_pairs.contains(p.label),
                })
                .collect(),
            capital_deployed_usd,
            capital_available_usd: (config.max_capital_usd - capital_deployed_usd).max(0.0),
            realized_pnl_usd: *self.realized_pnl_usd.lock().expect("lock poisoned"),
            positions: positions
                .values()
                .map(|p| LivePositionSnapshot {
                    pair_label: self.pair_label(&p.pair),
                    quantity_raw: p.quantity_raw,
                    entry_price: p.entry_price,
                    current_price: p.current_price,
                    allocated_usd: p.allocated_usd,
                    unrealized_pnl_usd: p.allocated_usd * (p.current_price / p.entry_price - 1.0),
                })
                .collect(),
        }
    }

    fn emit(&self, event: LiveEvent) {
        let _ = self.events.send(event);
    }

    /// Run forever, ticking on `config.poll_interval`.
    pub async fn run(&self) {
        loop {
            let interval = self
                .config
                .lock()
                .expect("config lock poisoned")
                .poll_interval;
            tokio::time::sleep(interval).await;
            if let Err(e) = self.tick().await {
                warn!("Live trading tick failed: {}", e);
            }
        }
    }

    pub async fn tick(&self) -> ExecutionResult<Vec<Signal>> {
        let snapshot = self.sample_market().await?;
        self.evaluate_stop_losses(&snapshot).await;
        self.evaluate_cross_dex_arbitrage().await;

        let strategies_enabled = self
            .config
            .lock()
            .expect("config lock poisoned")
            .strategies_enabled;
        let portfolio_state = self.portfolio_state();
        let signals = if strategies_enabled {
            self.strategy_manager
                .evaluate_all(&snapshot, &portfolio_state)
                .await
        } else {
            Vec::new()
        };

        for signal in &signals {
            if let Some(pair) = signal_pair(signal) {
                self.emit(LiveEvent::SignalGenerated {
                    strategy: signal.strategy.clone(),
                    pair_label: self.pair_label(&pair),
                    confidence: signal.confidence,
                });
            }
            self.act_on_signal(signal, &snapshot).await;
        }

        self.emit(LiveEvent::TickCompleted {
            timestamp: Utc::now(),
            signal_count: signals.len(),
        });

        Ok(signals)
    }

    /// Samples each registered DEX *individually* (Jupiter always,
    /// Orca/Raydium wherever a pair has a known pool) rather than just the
    /// aggregator's single best-route price -- `SpreadArbitrageStrategy`
    /// needs more than one source's `Price` per pair in the snapshot to
    /// detect a spread at all; collapsing to one "best" number the way
    /// execution does would make it permanently silent. Mirrors
    /// `solstice_simulation::PaperTradingEngine::sample_market`'s pattern.
    async fn sample_market(&self) -> ExecutionResult<MarketSnapshot> {
        let mut snapshot = MarketSnapshot::new(0);

        for pair in &self.pairs {
            let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);

            let slippage_bps = {
                let config = self.config.lock().expect("config lock poisoned");
                if config.disabled_pairs.contains(pair.label)
                    && !self
                        .positions
                        .lock()
                        .expect("positions lock poisoned")
                        .contains_key(&token_pair)
                {
                    continue;
                }
                config.slippage_bps
            };
            let quote_request = QuoteRequest::new(
                pair.base_mint,
                pair.quote_mint,
                pair.reference_amount,
                slippage_bps,
            );

            let mut observations = Vec::with_capacity(3);
            for (name, has_pool) in [
                ("Jupiter", true),
                ("Raydium", pair.raydium_pool.is_some()),
                ("Orca", pair.orca_pool.is_some()),
            ] {
                if !has_pool {
                    continue;
                }
                let Ok(client) = self.dex.get_client(name) else {
                    continue;
                };
                match client.get_quote(&quote_request).await {
                    Ok(quote) => {
                        let price = pair_price(pair, &quote);
                        self.emit(LiveEvent::PriceUpdate {
                            pair_label: format!("{} ({name})", pair.label),
                            price,
                            timestamp: Utc::now(),
                        });
                        observations.push(solstice_core::types::Price {
                            value: price,
                            pair: token_pair,
                            timestamp: Utc::now(),
                            confidence: 0.9,
                        });
                    }
                    Err(e) => warn!("[{}] {name} quote failed: {}", pair.label, e),
                }
            }

            if let Some(latest) = observations.last() {
                if let Some(position) = self
                    .positions
                    .lock()
                    .expect("lock poisoned")
                    .get_mut(&token_pair)
                {
                    position.current_price = latest.value;
                }
            }

            if !observations.is_empty() {
                snapshot.prices.insert(token_pair, observations);
            }
        }

        Ok(snapshot)
    }

    fn portfolio_state(&self) -> PortfolioState {
        let config = self.config.lock().expect("config lock poisoned");
        let capital_deployed_usd = *self.capital_deployed_usd.lock().expect("lock poisoned");
        let positions = self.positions.lock().expect("positions lock poisoned");

        PortfolioState {
            timestamp: Utc::now(),
            positions: positions
                .values()
                .map(|p| Position {
                    id: p.id,
                    pair: p.pair,
                    quantity: p.quantity_raw as i64,
                    entry_price: p.entry_price,
                    current_price: p.current_price,
                    opened_at: p.opened_at,
                    close_at: None,
                })
                .collect(),
            total_value_usd: config.max_capital_usd,
            available_capital: ((config.max_capital_usd - capital_deployed_usd).max(0.0)) as u64,
            risk_metrics: RiskMetrics::default(),
        }
    }

    /// Pure decision logic: given a signal and current market/portfolio
    /// state, decide whether and how much to trade. No I/O, so this is
    /// fully unit-testable without a live network.
    fn plan_signal(
        &self,
        signal: &Signal,
        snapshot: &MarketSnapshot,
    ) -> Result<PlannedTrade, String> {
        let pair =
            signal_pair(signal).ok_or_else(|| "signal has no associated pair".to_string())?;
        let price = snapshot
            .best_price(&pair)
            .ok_or_else(|| "no live price available for pair".to_string())?;

        let min_confidence = self
            .config
            .lock()
            .expect("config lock poisoned")
            .min_confidence;
        if signal.confidence < min_confidence {
            return Err(format!(
                "{} signal confidence {:.0}% below minimum {:.0}% to act",
                self.pair_label(&pair),
                signal.confidence * 100.0,
                min_confidence * 100.0
            ));
        }

        let is_buy = !matches!(signal.signal_type, SignalType::Sell { .. });

        if !is_buy {
            // A sell only reduces exposure, so (unlike a buy) it isn't
            // gated by the capital/position cap -- but this engine can't
            // short, so it can only sell what an open position already
            // holds. `execute_planned_trade` sells the *entire* held
            // quantity (a full close, mirroring `close_position`'s
            // stop-loss path) rather than sizing a partial amount here,
            // since sizing a sell in USD terms the same way as a buy
            // would risk trying to sell more base token than is actually
            // held.
            let _ = price;
            let has_position = self
                .positions
                .lock()
                .expect("positions lock poisoned")
                .contains_key(&pair);
            return if has_position {
                Ok(PlannedTrade {
                    pair,
                    pair_label: self.pair_label(&pair),
                    is_buy: false,
                    size_usd: 0,
                })
            } else {
                Err(format!(
                    "{} sell signal but no open position to sell",
                    self.pair_label(&pair)
                ))
            };
        }

        let config = self.config.lock().expect("config lock poisoned");
        let capital_deployed_usd = *self.capital_deployed_usd.lock().expect("lock poisoned");
        let positions = self.positions.lock().expect("positions lock poisoned");

        let existing_allocated_usd = positions.get(&pair).map(|p| p.allocated_usd).unwrap_or(0.0);
        let remaining_headroom = (config.risk_limits.position.max_single_position_usd as f64
            - existing_allocated_usd)
            .min(config.max_capital_usd - capital_deployed_usd)
            .max(0.0);
        drop(positions);

        if remaining_headroom <= 0.0 {
            return Err(format!(
                "{} already at position/capital cap",
                self.pair_label(&pair)
            ));
        }

        let risk_params = RiskParams {
            portfolio_value_usd: config.max_capital_usd,
            available_capital_usd: (config.max_capital_usd - capital_deployed_usd).max(0.0),
            max_position_usd: remaining_headroom,
            max_position_percent: config.risk_limits.position.max_position_percent,
            kelly_fraction: config.kelly_fraction,
            default_win_loss_ratio: config.default_win_loss_ratio,
        };

        let size_usd = PositionSizer::calculate_size(signal, &risk_params)
            .map_err(|e| format!("sizing failed: {e}"))?;

        let total_exposure_usd = capital_deployed_usd as u64;
        let approval = self.risk_checker.check_before_trade(
            size_usd,
            config.max_capital_usd as u64,
            self.positions.lock().expect("lock poisoned").len(),
            total_exposure_usd,
            0,
            Some(0.005),
        );
        if !matches!(approval, TradeApproval::Approved) {
            return Err(format!("risk check rejected: {approval:?}"));
        }

        let _ = price;
        Ok(PlannedTrade {
            pair,
            pair_label: self.pair_label(&pair),
            is_buy: true,
            size_usd,
        })
    }

    async fn act_on_signal(&self, signal: &Signal, snapshot: &MarketSnapshot) {
        let Some(pair) = signal_pair(signal) else {
            return;
        };

        let planned = match self.plan_signal(signal, snapshot) {
            Ok(planned) => planned,
            Err(reason) => {
                self.emit(LiveEvent::SignalSkipped {
                    strategy: signal.strategy.clone(),
                    pair_label: self.pair_label(&pair),
                    reason,
                });
                return;
            }
        };

        if !self.is_enabled() {
            self.emit(LiveEvent::WouldTrade {
                strategy: signal.strategy.clone(),
                pair_label: planned.pair_label.clone(),
                size_usd: planned.size_usd,
                is_buy: planned.is_buy,
            });
            return;
        }

        self.execute_planned_trade(signal, &planned).await;
    }

    async fn execute_planned_trade(&self, signal: &Signal, planned: &PlannedTrade) {
        let live_pair = self
            .pairs
            .iter()
            .find(|p| p.base_mint == planned.pair.base && p.quote_mint == planned.pair.quote);
        let Some(live_pair) = live_pair else {
            self.emit(LiveEvent::OrderFailed {
                strategy: signal.strategy.clone(),
                pair_label: planned.pair_label.clone(),
                reason: "pair not configured for live trading".to_string(),
            });
            return;
        };

        let slippage_bps = self
            .config
            .lock()
            .expect("config lock poisoned")
            .slippage_bps;

        // A buy spends the quote token (sized in USD, converted to the
        // quote mint's raw units) to acquire the base token. A sell does
        // the reverse -- but is sized by the base token actually held in
        // the open position, not by an independent USD figure, since this
        // engine can't sell base token it doesn't have. `plan_signal`
        // already confirmed a position exists for a sell signal before
        // getting here.
        let swap = if planned.is_buy {
            let quote_amount_raw = (planned.size_usd as f64
                * 10f64.powi(live_pair.quote_decimals as i32))
            .round() as u64;
            SwapRequest {
                input_mint: live_pair.quote_mint,
                output_mint: live_pair.base_mint,
                amount: quote_amount_raw,
                payer: self.wallet_pubkey,
                slippage_bps,
            }
        } else {
            let held_quantity_raw = self
                .positions
                .lock()
                .expect("lock poisoned")
                .get(&planned.pair)
                .map(|p| p.quantity_raw)
                .unwrap_or(0);
            if held_quantity_raw == 0 {
                self.emit(LiveEvent::OrderFailed {
                    strategy: signal.strategy.clone(),
                    pair_label: planned.pair_label.clone(),
                    reason: "sell signal but no open position to sell".to_string(),
                });
                return;
            }
            SwapRequest {
                input_mint: live_pair.base_mint,
                output_mint: live_pair.quote_mint,
                amount: held_quantity_raw,
                payer: self.wallet_pubkey,
                slippage_bps,
            }
        };

        // Load the wallet key and resolve the tip account (cached -- see
        // `cached_tip_accounts`) *before* fetching the quote we'll actually
        // trade against, so as little time as possible elapses between
        // reading that quote and submitting the swap built from it. A
        // stale quote combined with a tight slippage tolerance is a likely
        // cause of on-chain reverts.
        let keypair = match self.wallet_file.load_keypair() {
            Ok(kp) => kp,
            Err(e) => {
                self.emit(LiveEvent::OrderFailed {
                    strategy: signal.strategy.clone(),
                    pair_label: planned.pair_label.clone(),
                    reason: format!("failed to load wallet key: {e}"),
                });
                return;
            }
        };

        let tip_lamports = self
            .config
            .lock()
            .expect("config lock poisoned")
            .tip_lamports;
        let tip = match tip_lamports {
            Some(lamports) => {
                let accounts = self.cached_tip_accounts().await;
                accounts.first().map(|&account| (account, lamports))
            }
            None => None,
        };

        // Best-price route across every registered DEX (Jupiter, plus
        // Orca/Raydium for pairs with a known pool -- see
        // `LiveTradedPair::orca_pool`/`raydium_pool`), not just Jupiter's
        // own aggregated route -- occasionally a direct pool beats
        // Jupiter's routing overhead, and this is a fallback if Jupiter's
        // API has an outage.
        let (winning_name, quote) = match self
            .dex
            .get_best_route_with_source(&QuoteRequest::new(
                swap.input_mint,
                swap.output_mint,
                swap.amount,
                slippage_bps,
            ))
            .await
        {
            Ok(result) => result,
            Err(e) => {
                self.emit(LiveEvent::OrderFailed {
                    strategy: signal.strategy.clone(),
                    pair_label: planned.pair_label.clone(),
                    reason: format!("failed to fetch execution quote: {e}"),
                });
                return;
            }
        };

        let winning_dex = match self.dex.get_client(&winning_name) {
            Ok(client) => client,
            Err(e) => {
                self.emit(LiveEvent::OrderFailed {
                    strategy: signal.strategy.clone(),
                    pair_label: planned.pair_label.clone(),
                    reason: format!("failed to resolve winning route's DEX client: {e}"),
                });
                return;
            }
        };

        let outcome = execute_swap(
            &self.jito,
            &self.rpc,
            winning_dex.as_ref(),
            &swap,
            &quote,
            &keypair,
            tip,
            Duration::from_secs(60),
            Duration::from_secs(2),
        )
        .await;

        match outcome {
            Ok(outcome) => {
                let price = if planned.is_buy {
                    quote_price(live_pair, &quote)
                } else {
                    sell_price(live_pair, &quote)
                };
                let size_usd = self.record_fill(planned, live_pair, &quote, price);
                self.emit(LiveEvent::OrderFilled {
                    strategy: signal.strategy.clone(),
                    pair_label: planned.pair_label.clone(),
                    size_usd,
                    price,
                    method: format!("{:?}", outcome.method),
                    signature: outcome.signatures.first().map(|s| s.to_string()),
                });
            }
            Err(e) => {
                self.emit(LiveEvent::OrderFailed {
                    strategy: signal.strategy.clone(),
                    pair_label: planned.pair_label.clone(),
                    reason: e.to_string(),
                });
            }
        }
    }

    /// Update position/capital bookkeeping for a landed fill and return
    /// its actual USD size -- for a buy this is `planned.size_usd` (known
    /// before the swap); for a sell it's only known from the quote's
    /// proceeds, since the amount sold was sized in base-token terms (the
    /// full held quantity), not USD.
    fn record_fill(
        &self,
        planned: &PlannedTrade,
        live_pair: &LiveTradedPair,
        quote: &Quote,
        price: f64,
    ) -> u64 {
        if planned.is_buy {
            let quantity_raw = quote.out_amount;
            *self.capital_deployed_usd.lock().expect("lock poisoned") += planned.size_usd as f64;

            let plan = ExecutionPlan {
                signal: Signal::new(
                    "live".to_string(),
                    SignalType::Buy { pair: planned.pair },
                    1.0,
                ),
                pair: planned.pair,
                quote: quote.clone(),
                size_usd: planned.size_usd,
                approval: TradeApproval::Approved,
            };
            if let Ok(order_id) = self.order_manager.submit(plan) {
                let _ = self.order_manager.record_fill(
                    &order_id,
                    Fill {
                        amount: planned.size_usd,
                        price,
                        fee: 0.0,
                        timestamp: Utc::now(),
                        tx_signature: None,
                    },
                );
            }

            self.positions
                .lock()
                .expect("lock poisoned")
                .entry(planned.pair)
                .and_modify(|p| {
                    p.quantity_raw = p.quantity_raw.saturating_add(quantity_raw);
                    p.allocated_usd += planned.size_usd as f64;
                    p.current_price = price;
                })
                .or_insert_with(|| LivePosition {
                    id: PositionId::new(),
                    pair: planned.pair,
                    quantity_raw,
                    entry_price: price,
                    current_price: price,
                    allocated_usd: planned.size_usd as f64,
                    opened_at: Utc::now(),
                });

            planned.size_usd
        } else {
            // A sell here is always a full close (see `plan_signal`/
            // `execute_planned_trade`): the entire held quantity was sold,
            // so the position is removed rather than decremented, mirroring
            // `close_position`'s stop-loss path.
            let allocated_usd = self
                .positions
                .lock()
                .expect("lock poisoned")
                .remove(&planned.pair)
                .map(|p| p.allocated_usd)
                .unwrap_or(0.0);
            let exit_value_usd =
                quote.out_amount as f64 / 10f64.powi(live_pair.quote_decimals as i32);
            let realized_pnl = exit_value_usd - allocated_usd;
            *self.realized_pnl_usd.lock().expect("lock poisoned") += realized_pnl;
            *self.capital_deployed_usd.lock().expect("lock poisoned") -= allocated_usd;

            let plan = ExecutionPlan {
                signal: Signal::new(
                    "live".to_string(),
                    SignalType::Sell { pair: planned.pair },
                    1.0,
                ),
                pair: planned.pair,
                quote: quote.clone(),
                size_usd: exit_value_usd.round() as u64,
                approval: TradeApproval::Approved,
            };
            if let Ok(order_id) = self.order_manager.submit(plan) {
                let _ = self.order_manager.record_fill(
                    &order_id,
                    Fill {
                        amount: exit_value_usd.round() as u64,
                        price,
                        fee: 0.0,
                        timestamp: Utc::now(),
                        tx_signature: None,
                    },
                );
            }

            exit_value_usd.round() as u64
        }
    }

    /// Evaluates both exit paths for every open position: stop-loss (fixed
    /// at construction, like `risk_limits`) and take-profit (read fresh
    /// from config each tick so `set_take_profit_percent` takes effect
    /// immediately, matching `min_confidence`'s pattern). Neither SMA nor
    /// SpreadArb ever emits its own exit signal, so this is the only path
    /// a profitable position closes through.
    async fn evaluate_stop_losses(&self, _snapshot: &MarketSnapshot) {
        let portfolio_state = self.portfolio_state();

        let stop_triggers = self.stop_loss.evaluate_stops(&portfolio_state.positions);
        for trigger in stop_triggers {
            self.close_position(trigger.position_id, trigger.reason)
                .await;
        }

        let take_profit_percent = self
            .config
            .lock()
            .expect("config lock poisoned")
            .take_profit_percent;
        let profit_triggers = TakeProfitManager::new(take_profit_percent)
            .evaluate_targets(&portfolio_state.positions);
        for trigger in profit_triggers {
            self.close_position(trigger.position_id, trigger.reason)
                .await;
        }
    }

    /// Quotes every registered DEX individually for `pair` (like
    /// `sample_market`, but keeping the DEX name attached to each price
    /// instead of collapsing to an untagged `Price` list) and returns the
    /// cheapest-vs-priciest gap, if any two distinct DEXes both quoted.
    async fn find_arb_opportunity(&self, pair: &LiveTradedPair) -> Option<ArbOpportunity> {
        let slippage_bps = self
            .config
            .lock()
            .expect("config lock poisoned")
            .cross_dex_max_slippage_bps;

        let mut observations: Vec<(String, f64)> = Vec::with_capacity(3);
        for (name, has_pool) in [
            ("Jupiter", true),
            ("Raydium", pair.raydium_pool.is_some()),
            ("Orca", pair.orca_pool.is_some()),
        ] {
            if !has_pool {
                continue;
            }
            let Ok(client) = self.dex.get_client(name) else {
                continue;
            };
            let request = QuoteRequest::new(
                pair.base_mint,
                pair.quote_mint,
                pair.reference_amount,
                slippage_bps,
            );
            if let Ok(quote) = client.get_quote(&request).await {
                observations.push((name.to_string(), pair_price(pair, &quote)));
            }
        }

        if observations.len() < 2 {
            return None;
        }

        let min = observations
            .iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        let max = observations
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;

        if min.0 == max.0 || min.1 <= 0.0 {
            return None;
        }

        Some(ArbOpportunity {
            buy_dex: min.0.clone(),
            sell_dex: max.0.clone(),
            buy_price: min.1,
            sell_price: max.1,
            spread: (max.1 - min.1) / min.1,
        })
    }

    /// Runs the cross-DEX arbitrage executor over every configured pair,
    /// if armed (`cross_dex_arb_enabled`). The intent is for capital to
    /// keep cycling between quote and base rather than sitting still:
    /// each pair is either flat (no tracked position, so this looks for
    /// a fresh buy-low/sell-high opportunity) or holding inventory (a
    /// buy whose sell leg hasn't landed yet -- from this tick's own
    /// attempt, a prior failed sell leg, or an on-chain balance adopted
    /// by `reconcile_untracked_balance` below), in which case this keeps
    /// retrying the flatten-back-to-quote every tick via the existing
    /// generic `close_position` (the same path stop-loss/take-profit
    /// use) rather than leaving it stuck until some price threshold is
    /// crossed.
    async fn evaluate_cross_dex_arbitrage(&self) {
        let (cross_dex_arb_enabled, min_spread, max_slippage_bps, min_net_edge_bps) = {
            let config = self.config.lock().expect("config lock poisoned");
            (
                config.cross_dex_arb_enabled,
                config.cross_dex_min_spread,
                config.cross_dex_max_slippage_bps,
                config.cross_dex_min_net_edge_bps,
            )
        };
        if !cross_dex_arb_enabled {
            return;
        }

        let required_spread =
            required_cross_dex_spread(min_spread, max_slippage_bps, min_net_edge_bps);

        for pair in self.pairs.iter().copied() {
            let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);

            self.reconcile_untracked_balance(&pair).await;

            let existing_position = self
                .positions
                .lock()
                .expect("positions lock poisoned")
                .get(&token_pair)
                .map(|p| p.id);
            if let Some(position_id) = existing_position {
                self.close_position(
                    position_id,
                    "cross-dex arb: cycling capital back to quote".to_string(),
                )
                .await;
                continue;
            }

            if self
                .config
                .lock()
                .expect("config lock poisoned")
                .disabled_pairs
                .contains(pair.label)
            {
                continue;
            }

            let Some(opportunity) = self.find_arb_opportunity(&pair).await else {
                continue;
            };
            if opportunity.spread < required_spread {
                continue;
            }

            self.emit(LiveEvent::CrossDexOpportunityDetected {
                pair_label: pair.label.to_string(),
                buy_dex: opportunity.buy_dex.clone(),
                sell_dex: opportunity.sell_dex.clone(),
                buy_price: opportunity.buy_price,
                sell_price: opportunity.sell_price,
                spread_percent: opportunity.spread * 100.0,
            });
            self.execute_cross_dex_arb(&pair, opportunity).await;
        }
    }

    /// Adopts an untracked on-chain `pair.base_mint` balance as a tracked
    /// position, if one exists beyond dust. No-op if a position is
    /// already tracked for this pair -- this only recovers from gaps
    /// (inventory left by a failed sell leg or, before the balance-read
    /// retry fix, a buy whose confirmation raced a lagging RPC read),
    /// never double-counts. The entry price is estimated from the
    /// current best quote, not the real historical fill -- that isn't
    /// reliably recoverable here, and for the purpose of flattening this
    /// back to quote (via `close_position` in the caller) an approximate
    /// entry is enough; it only affects realized-P&L bookkeeping and
    /// stop-loss/take-profit timing until the flatten attempt lands.
    async fn reconcile_untracked_balance(&self, pair: &LiveTradedPair) {
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        if self
            .positions
            .lock()
            .expect("positions lock poisoned")
            .contains_key(&token_pair)
        {
            return;
        }

        let Ok(balance_raw) = self
            .rpc
            .get_token_balance(&self.wallet_pubkey, &pair.base_mint)
            .await
        else {
            return;
        };
        // Ignore dust (rent-exempt reserve noise, rounding) so this
        // doesn't spawn a phantom position worth a fraction of a cent.
        let dust_threshold_raw = 10u64.pow(pair.base_decimals as u32) / 1000;
        if balance_raw <= dust_threshold_raw {
            return;
        }

        let slippage_bps = self
            .config
            .lock()
            .expect("config lock poisoned")
            .cross_dex_max_slippage_bps;
        let Ok(client) = self.dex.get_client("Jupiter") else {
            return;
        };
        let request = QuoteRequest::new(
            pair.base_mint,
            pair.quote_mint,
            pair.reference_amount,
            slippage_bps,
        );
        let Ok(quote) = client.get_quote(&request).await else {
            return;
        };
        let price = pair_price(pair, &quote);
        if price <= 0.0 {
            return;
        }

        let quantity = balance_raw as f64 / 10f64.powi(pair.base_decimals as i32);
        let allocated_usd = quantity * price;

        self.positions
            .lock()
            .expect("positions lock poisoned")
            .insert(
                token_pair,
                LivePosition {
                    id: PositionId::new(),
                    pair: token_pair,
                    quantity_raw: balance_raw,
                    entry_price: price,
                    current_price: price,
                    allocated_usd,
                    opened_at: Utc::now(),
                },
            );
        *self.capital_deployed_usd.lock().expect("lock poisoned") += allocated_usd;

        self.emit(LiveEvent::UntrackedBalanceAdopted {
            pair_label: pair.label.to_string(),
            quantity,
            estimated_usd: allocated_usd,
        });
    }

    /// Buys `pair.base_mint` on `opportunity.buy_dex` then immediately
    /// sells the received quantity on `opportunity.sell_dex`. **Not
    /// atomic** -- these are two separate transactions, so a real price
    /// move between them (or an outright failure of the second leg) is
    /// possible; see `LiveTradingConfig::cross_dex_arb_enabled`. If the
    /// sell leg fails after the buy leg lands, the bought inventory is
    /// registered as a normal tracked position (protected by
    /// stop-loss/take-profit from that point on) rather than left an
    /// untracked wallet balance.
    async fn execute_cross_dex_arb(&self, pair: &LiveTradedPair, opportunity: ArbOpportunity) {
        let pair_label = pair.label.to_string();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);

        if !self.is_enabled() {
            self.emit(LiveEvent::WouldTrade {
                strategy: "CrossDexArb".to_string(),
                pair_label,
                size_usd: 0,
                is_buy: true,
            });
            return;
        }

        let (configured_size_usd, min_position_size_usd, slippage_bps, tip_lamports) = {
            let config = self.config.lock().expect("config lock poisoned");
            let capital_deployed_usd = *self.capital_deployed_usd.lock().expect("lock poisoned");
            let remaining_headroom = (config.risk_limits.position.max_single_position_usd as f64)
                .min(config.max_capital_usd - capital_deployed_usd)
                .max(0.0);
            (
                remaining_headroom,
                config.risk_limits.position.min_position_size_usd as f64,
                config.cross_dex_max_slippage_bps,
                config.tip_lamports,
            )
        };

        // `configured_size_usd` comes from internal bookkeeping
        // (`capital_deployed_usd`), which can drift from on-chain reality
        // -- e.g. a prior buy leg that landed but wasn't tracked (see the
        // balance-delta retry below) leaves `capital_deployed_usd`
        // understating what's actually already committed. Capping against
        // the wallet's real quote-token balance means a stale bookkeeping
        // number can only make this *skip* a trade it shouldn't attempt,
        // never submit one the wallet can't actually cover.
        let quote_balance_raw = self
            .rpc
            .get_token_balance(&self.wallet_pubkey, &pair.quote_mint)
            .await
            .unwrap_or(0);
        let quote_balance_usd = quote_balance_raw as f64 / 10f64.powi(pair.quote_decimals as i32);
        let size_usd = configured_size_usd.min(quote_balance_usd);

        if size_usd < min_position_size_usd {
            self.emit(LiveEvent::CrossDexArbFailed {
                pair_label,
                leg: "sizing".to_string(),
                reason: format!(
                    "insufficient capital for a cross-DEX arb trade (configured headroom {configured_size_usd:.2}, actual wallet balance {quote_balance_usd:.2})"
                ),
            });
            return;
        }
        let size_usd = size_usd.floor() as u64;

        let keypair = match self.wallet_file.load_keypair() {
            Ok(kp) => kp,
            Err(e) => {
                self.emit(LiveEvent::CrossDexArbFailed {
                    pair_label,
                    leg: "buy".to_string(),
                    reason: format!("failed to load wallet key: {e}"),
                });
                return;
            }
        };

        let tip = match tip_lamports {
            Some(lamports) => {
                let accounts = self.cached_tip_accounts().await;
                accounts.first().map(|&account| (account, lamports))
            }
            None => None,
        };

        // --- Leg 1: buy on the cheaper DEX ---
        let Ok(buy_client) = self.dex.get_client(&opportunity.buy_dex) else {
            self.emit(LiveEvent::CrossDexArbFailed {
                pair_label,
                leg: "buy".to_string(),
                reason: format!("failed to resolve {} client", opportunity.buy_dex),
            });
            return;
        };

        let quote_amount_raw =
            (size_usd as f64 * 10f64.powi(pair.quote_decimals as i32)).round() as u64;
        let buy_swap = SwapRequest {
            input_mint: pair.quote_mint,
            output_mint: pair.base_mint,
            amount: quote_amount_raw,
            payer: self.wallet_pubkey,
            slippage_bps,
        };

        let buy_quote = match buy_client
            .get_quote(&QuoteRequest::new(
                buy_swap.input_mint,
                buy_swap.output_mint,
                buy_swap.amount,
                slippage_bps,
            ))
            .await
        {
            Ok(q) => q,
            Err(e) => {
                self.emit(LiveEvent::CrossDexArbFailed {
                    pair_label,
                    leg: "buy".to_string(),
                    reason: format!(
                        "failed to fetch buy quote from {}: {e}",
                        opportunity.buy_dex
                    ),
                });
                return;
            }
        };

        let base_balance_before = self
            .rpc
            .get_token_balance(&self.wallet_pubkey, &pair.base_mint)
            .await
            .unwrap_or(0);

        let buy_outcome = execute_swap(
            &self.jito,
            &self.rpc,
            buy_client.as_ref(),
            &buy_swap,
            &buy_quote,
            &keypair,
            tip,
            Duration::from_secs(60),
            Duration::from_secs(2),
        )
        .await;

        let buy_outcome = match buy_outcome {
            Ok(o) => o,
            Err(e) => {
                self.emit(LiveEvent::CrossDexArbFailed {
                    pair_label,
                    leg: "buy".to_string(),
                    reason: e.to_string(),
                });
                return;
            }
        };

        // Leg 1 landed. Read the actual received quantity from the
        // balance delta rather than trusting `buy_quote.out_amount` --
        // the two legs are separate transactions, not atomic, so the
        // real fill can differ from the quote. A single immediate read
        // isn't reliable: a load-balanced RPC provider can answer from a
        // node that hasn't yet caught up to the just-landed transaction,
        // reading back a stale (pre-buy) balance even though the swap
        // genuinely succeeded -- confirmed directly against a real
        // transaction that landed a buy correctly but was reported as
        // failed for exactly this reason. Retry briefly before concluding
        // the buy produced nothing.
        let mut received_quantity = 0u64;
        for attempt in 0..5 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let base_balance_after = match self
                .rpc
                .get_token_balance(&self.wallet_pubkey, &pair.base_mint)
                .await
            {
                Ok(b) => b,
                Err(e) => {
                    warn!(
                        "failed to read post-buy balance for {} (attempt {}): {}",
                        pair_label,
                        attempt + 1,
                        e
                    );
                    continue;
                }
            };
            received_quantity = base_balance_after.saturating_sub(base_balance_before);
            if received_quantity > 0 {
                break;
            }
        }
        if received_quantity == 0 {
            self.emit(LiveEvent::CrossDexArbFailed {
                pair_label,
                leg: "buy".to_string(),
                reason: "buy leg landed but no base token balance increase was observed after \
                         retrying -- if the wallet's actual balance did increase, this was a \
                         read-consistency failure, not a lost trade; check on-chain state before \
                         assuming funds are missing"
                    .to_string(),
            });
            return;
        }

        let buy_price = quote_price(pair, &buy_quote);

        // Track this as an open position immediately, before attempting
        // the sell leg -- if that leg fails below, this inventory must
        // stay visible and protected by stop-loss/take-profit rather
        // than becoming an untracked wallet balance.
        *self.capital_deployed_usd.lock().expect("lock poisoned") += size_usd as f64;
        self.positions.lock().expect("lock poisoned").insert(
            token_pair,
            LivePosition {
                id: PositionId::new(),
                pair: token_pair,
                quantity_raw: received_quantity,
                entry_price: buy_price,
                current_price: buy_price,
                allocated_usd: size_usd as f64,
                opened_at: Utc::now(),
            },
        );

        // --- Leg 2: sell on the pricier DEX ---
        let Ok(sell_client) = self.dex.get_client(&opportunity.sell_dex) else {
            self.emit(LiveEvent::CrossDexArbFailed {
                pair_label,
                leg: "sell".to_string(),
                reason: format!(
                    "failed to resolve {} client -- position tracked, protected by stop-loss/take-profit",
                    opportunity.sell_dex
                ),
            });
            return;
        };

        let sell_swap = SwapRequest {
            input_mint: pair.base_mint,
            output_mint: pair.quote_mint,
            amount: received_quantity,
            payer: self.wallet_pubkey,
            slippage_bps,
        };

        let sell_quote = match sell_client
            .get_quote(&QuoteRequest::new(
                sell_swap.input_mint,
                sell_swap.output_mint,
                sell_swap.amount,
                slippage_bps,
            ))
            .await
        {
            Ok(q) => q,
            Err(e) => {
                self.emit(LiveEvent::CrossDexArbFailed {
                    pair_label,
                    leg: "sell".to_string(),
                    reason: format!(
                        "failed to fetch sell quote from {}: {e} -- position tracked, protected by stop-loss/take-profit",
                        opportunity.sell_dex
                    ),
                });
                return;
            }
        };

        let sell_outcome = execute_swap(
            &self.jito,
            &self.rpc,
            sell_client.as_ref(),
            &sell_swap,
            &sell_quote,
            &keypair,
            tip,
            Duration::from_secs(60),
            Duration::from_secs(2),
        )
        .await;

        match sell_outcome {
            Ok(outcome) => {
                let realized_sell_price = sell_price(pair, &sell_quote);
                let exit_value_usd =
                    sell_quote.out_amount as f64 / 10f64.powi(pair.quote_decimals as i32);
                let realized_pnl = exit_value_usd - size_usd as f64;

                self.positions
                    .lock()
                    .expect("lock poisoned")
                    .remove(&token_pair);
                *self.capital_deployed_usd.lock().expect("lock poisoned") -= size_usd as f64;
                *self.realized_pnl_usd.lock().expect("lock poisoned") += realized_pnl;

                self.emit(LiveEvent::CrossDexArbFilled {
                    pair_label,
                    buy_dex: opportunity.buy_dex.clone(),
                    sell_dex: opportunity.sell_dex.clone(),
                    size_usd,
                    buy_price,
                    sell_price: realized_sell_price,
                    realized_pnl_usd: realized_pnl,
                    buy_signature: buy_outcome.signatures.first().map(|s| s.to_string()),
                    sell_signature: outcome.signatures.first().map(|s| s.to_string()),
                });
            }
            Err(e) => {
                self.emit(LiveEvent::CrossDexArbFailed {
                    pair_label,
                    leg: "sell".to_string(),
                    reason: format!("{e} -- position tracked, protected by stop-loss/take-profit"),
                });
            }
        }
    }

    async fn close_position(&self, id: PositionId, reason: String) {
        let position_info = {
            let positions = self.positions.lock().expect("lock poisoned");
            positions
                .iter()
                .find(|(_, p)| p.id == id)
                .map(|(pair, p)| (*pair, p.quantity_raw, p.allocated_usd))
        };
        let Some((pair, quantity_raw, allocated_usd)) = position_info else {
            return;
        };

        let pair_label = self.pair_label(&pair);

        if !self.is_enabled() {
            self.emit(LiveEvent::SignalSkipped {
                strategy: "stop-loss".to_string(),
                pair_label,
                reason: format!("would close position ({reason}), live trading disabled"),
            });
            return;
        }

        let live_pair = self
            .pairs
            .iter()
            .find(|p| p.base_mint == pair.base && p.quote_mint == pair.quote);
        let Some(live_pair) = live_pair else { return };

        let swap = SwapRequest {
            input_mint: pair.base,
            output_mint: pair.quote,
            amount: quantity_raw,
            payer: self.wallet_pubkey,
            slippage_bps: self
                .config
                .lock()
                .expect("config lock poisoned")
                .slippage_bps,
        };

        let (winning_name, quote) = match self
            .dex
            .get_best_route_with_source(&QuoteRequest::new(
                swap.input_mint,
                swap.output_mint,
                swap.amount,
                swap.slippage_bps,
            ))
            .await
        {
            Ok(result) => result,
            Err(e) => {
                warn!(
                    "failed to fetch stop-loss close quote for {}: {}",
                    pair_label, e
                );
                return;
            }
        };

        let winning_dex = match self.dex.get_client(&winning_name) {
            Ok(client) => client,
            Err(e) => {
                warn!(
                    "failed to resolve winning route's DEX client for stop-loss close of {}: {}",
                    pair_label, e
                );
                return;
            }
        };

        let keypair = match self.wallet_file.load_keypair() {
            Ok(kp) => kp,
            Err(e) => {
                warn!("failed to load wallet key for stop-loss close: {}", e);
                return;
            }
        };

        let outcome = execute_swap(
            &self.jito,
            &self.rpc,
            winning_dex.as_ref(),
            &swap,
            &quote,
            &keypair,
            None,
            Duration::from_secs(60),
            Duration::from_secs(2),
        )
        .await;

        match outcome {
            Ok(_) => {
                let exit_value_usd =
                    quote.out_amount as f64 / 10f64.powi(live_pair.quote_decimals as i32);
                let realized_pnl = exit_value_usd - allocated_usd;
                *self.realized_pnl_usd.lock().expect("lock poisoned") += realized_pnl;
                *self.capital_deployed_usd.lock().expect("lock poisoned") -= allocated_usd;
                self.positions.lock().expect("lock poisoned").remove(&pair);

                self.emit(LiveEvent::PositionClosed {
                    pair_label,
                    realized_pnl_usd: realized_pnl,
                    reason,
                });
            }
            Err(e) => {
                warn!("failed to close position for {}: {}", pair_label, e);
            }
        }
    }
}

fn pair_price(pair: &LiveTradedPair, quote: &Quote) -> f64 {
    let out = quote.out_amount as f64 / 10f64.powi(pair.quote_decimals as i32);
    let base_amount = pair.reference_amount as f64 / 10f64.powi(base_decimals_hint(pair) as i32);
    if base_amount <= 0.0 {
        0.0
    } else {
        out / base_amount
    }
}

fn quote_price(pair: &LiveTradedPair, quote: &Quote) -> f64 {
    // For a buy (quote -> base), price = quote spent / base received.
    let quote_amount = quote.in_amount as f64 / 10f64.powi(pair.quote_decimals as i32);
    let base_amount = quote.out_amount as f64 / 10f64.powi(base_decimals_hint(pair) as i32);
    if base_amount <= 0.0 {
        0.0
    } else {
        quote_amount / base_amount
    }
}

fn sell_price(pair: &LiveTradedPair, quote: &Quote) -> f64 {
    // For a sell (base -> quote), price = quote received / base spent.
    let quote_amount = quote.out_amount as f64 / 10f64.powi(pair.quote_decimals as i32);
    let base_amount = quote.in_amount as f64 / 10f64.powi(base_decimals_hint(pair) as i32);
    if base_amount <= 0.0 {
        0.0
    } else {
        quote_amount / base_amount
    }
}

fn base_decimals_hint(pair: &LiveTradedPair) -> u8 {
    pair.base_decimals
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::{
        ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits,
    };
    use solstice_strategy::StrategyConfig;
    use std::time::SystemTime;

    fn temp_wallet_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("solstice-live-engine-test-{name}-{nanos}.json"))
    }

    fn test_pair() -> LiveTradedPair {
        LiveTradedPair {
            label: "TEST/USDC",
            base_mint: Pubkey::new_unique(),
            base_decimals: 9,
            quote_mint: Pubkey::new_unique(),
            quote_decimals: 6,
            reference_amount: 10_000_000,
            raydium_pool: None,
            orca_pool: None,
        }
    }

    fn test_config(max_capital_usd: f64) -> LiveTradingConfig {
        LiveTradingConfig {
            max_capital_usd,
            min_confidence: 0.0,
            strategies_enabled: true,
            risk_limits: crate::risk::RiskLimits {
                position: PositionLimits {
                    max_single_position_usd: max_capital_usd as u64,
                    max_position_percent: 1.0,
                    min_position_size_usd: 1,
                    max_open_positions: 5,
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
                    max_order_size_usd: 1_000_000,
                    max_slippage_percent: 0.5,
                },
            },
            kelly_fraction: 0.5,
            default_win_loss_ratio: 2.0,
            stop_loss_percent: 0.1,
            take_profit_percent: 0.05,
            cross_dex_arb_enabled: false,
            cross_dex_min_spread: 0.015,
            cross_dex_max_slippage_bps: 30,
            cross_dex_min_net_edge_bps: 30,
            disabled_pairs: std::collections::HashSet::new(),
            slippage_bps: 50,
            poll_interval: Duration::from_secs(3600),
            tip_lamports: None,
        }
    }

    /// Builds a real `LiveTradingEngine` against a throwaway (never
    /// funded, never used) local wallet file and an unreachable RPC
    /// endpoint -- fine for testing `plan_signal`, `enable`/`disable`,
    /// and status reporting, none of which touch the network.
    fn test_engine(pair: LiveTradedPair, max_capital_usd: f64) -> LiveTradingEngine {
        let wallet_path = temp_wallet_path("engine");
        let wallet_file = WalletFile::at(&wallet_path);
        wallet_file.generate().unwrap();

        let rpc = Arc::new(
            SolanaRpcClient::with_endpoints(vec!["http://127.0.0.1:1".to_string()]).unwrap(),
        );
        let strategy_manager = Arc::new(StrategyManager::new(StrategyConfig::default()));

        let engine = LiveTradingEngine::new(
            wallet_file,
            rpc,
            strategy_manager,
            vec![pair],
            test_config(max_capital_usd),
        )
        .unwrap();

        std::fs::remove_file(&wallet_path).ok();
        engine
    }

    #[test]
    fn test_engine_constructs_with_orca_and_raydium_pools_registered() {
        // Exercises the constructor path that registers Orca/Raydium
        // pools into the aggregator (`LiveTradingEngine::new`'s
        // `raydium.register_pool`/`orca.register_pool` calls) -- this
        // should succeed and produce a normal, disabled-by-default engine
        // exactly like a pair with no pools set.
        let mut pair = test_pair();
        pair.raydium_pool = Some(Pubkey::new_unique());
        pair.orca_pool = Some(Pubkey::new_unique());

        let engine = test_engine(pair, 50.0);
        assert!(!engine.is_enabled());
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

    fn sell_signal(pair: TokenPair, confidence: f64) -> Signal {
        Signal::new("Test".to_string(), SignalType::Sell { pair }, confidence)
    }

    #[test]
    fn test_disabled_by_default() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert!(!engine.is_enabled());
    }

    #[test]
    fn test_enable_disable_toggle_and_events() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        let mut events = engine.subscribe();

        engine.enable();
        assert!(engine.is_enabled());
        assert!(matches!(
            events.try_recv(),
            Ok(LiveEvent::LiveTradingEnabled)
        ));

        engine.disable();
        assert!(!engine.is_enabled());
        assert!(matches!(
            events.try_recv(),
            Ok(LiveEvent::LiveTradingDisabled)
        ));
    }

    #[test]
    fn test_set_max_capital_usd_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert_eq!(engine.status().max_capital_usd, 50.0);

        engine.set_max_capital_usd(100.0);
        assert_eq!(engine.status().max_capital_usd, 100.0);
    }

    #[test]
    fn test_set_min_confidence_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);

        engine.set_min_confidence(0.8);
        assert_eq!(engine.status().min_confidence, 0.8);
    }

    #[test]
    fn test_plan_signal_rejects_below_min_confidence() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);
        engine.set_min_confidence(0.8);

        let snapshot = snapshot_with_price(token_pair, 100.0);
        let signal = buy_signal(token_pair, 0.65);

        let result = engine.plan_signal(&signal, &snapshot);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("below minimum"));
    }

    #[test]
    fn test_plan_signal_accepts_at_or_above_min_confidence() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);
        engine.set_min_confidence(0.8);

        let snapshot = snapshot_with_price(token_pair, 100.0);
        let signal = buy_signal(token_pair, 0.8);

        assert!(engine.plan_signal(&signal, &snapshot).is_ok());
    }

    #[test]
    fn test_plan_signal_sizes_within_max_capital() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);

        let snapshot = snapshot_with_price(token_pair, 100.0);
        let signal = buy_signal(token_pair, 0.95);

        let planned = engine.plan_signal(&signal, &snapshot).unwrap();
        assert!(planned.size_usd as f64 <= 50.0);
        assert!(planned.is_buy);
    }

    #[test]
    fn test_plan_signal_rejects_without_a_price() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);

        let empty_snapshot = MarketSnapshot::new(0);
        let signal = buy_signal(token_pair, 0.95);

        assert!(engine.plan_signal(&signal, &empty_snapshot).is_err());
    }

    #[test]
    fn test_plan_signal_rejects_when_capital_fully_deployed() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);

        *engine.capital_deployed_usd.lock().unwrap() = 50.0;

        let snapshot = snapshot_with_price(token_pair, 100.0);
        let signal = buy_signal(token_pair, 0.95);

        let result = engine.plan_signal(&signal, &snapshot);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cap"));
    }

    #[test]
    fn test_plan_signal_sell_rejects_without_a_position() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);

        let snapshot = snapshot_with_price(token_pair, 100.0);
        let signal = sell_signal(token_pair, 0.95);

        let result = engine.plan_signal(&signal, &snapshot);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no open position"));
    }

    #[test]
    fn test_plan_signal_sell_succeeds_with_a_position() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);

        engine.positions.lock().unwrap().insert(
            token_pair,
            LivePosition {
                id: PositionId::new(),
                pair: token_pair,
                quantity_raw: 1_000_000_000,
                entry_price: 100.0,
                current_price: 100.0,
                allocated_usd: 10.0,
                opened_at: Utc::now(),
            },
        );

        // Even fully capital-deployed, a sell must still be plannable --
        // it only reduces exposure, never opens new exposure.
        *engine.capital_deployed_usd.lock().unwrap() = 50.0;

        let snapshot = snapshot_with_price(token_pair, 100.0);
        let signal = sell_signal(token_pair, 0.95);

        let planned = engine.plan_signal(&signal, &snapshot).unwrap();
        assert!(!planned.is_buy);
    }

    #[test]
    fn test_status_reflects_capital_deployed_and_available() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);

        *engine.capital_deployed_usd.lock().unwrap() = 20.0;

        let status = engine.status();
        assert_eq!(status.capital_deployed_usd, 20.0);
        assert_eq!(status.capital_available_usd, 30.0);
    }

    #[tokio::test]
    async fn test_disabled_engine_never_touches_capital_on_would_trade() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);
        assert!(!engine.is_enabled());

        let snapshot = snapshot_with_price(token_pair, 100.0);
        let signal = buy_signal(token_pair, 0.95);
        let mut events = engine.subscribe();

        engine.act_on_signal(&signal, &snapshot).await;

        assert_eq!(*engine.capital_deployed_usd.lock().unwrap(), 0.0);
        let mut saw_would_trade = false;
        while let Ok(event) = events.try_recv() {
            if matches!(event, LiveEvent::WouldTrade { .. }) {
                saw_would_trade = true;
            }
        }
        assert!(
            saw_would_trade,
            "expected a WouldTrade event while disabled"
        );
    }

    #[test]
    fn test_set_take_profit_percent_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert_eq!(engine.status().take_profit_percent, 0.05);

        engine.set_take_profit_percent(0.1);
        assert_eq!(engine.status().take_profit_percent, 0.1);
    }

    #[test]
    fn test_set_strategies_enabled_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert!(engine.status().strategies_enabled);

        engine.set_strategies_enabled(false);
        assert!(!engine.status().strategies_enabled);
    }

    #[tokio::test]
    async fn test_evaluate_stop_losses_flags_position_beyond_take_profit() {
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);
        assert!(!engine.is_enabled());

        // +10% gain, above the 5% default take-profit target.
        engine.positions.lock().unwrap().insert(
            token_pair,
            LivePosition {
                id: PositionId::new(),
                pair: token_pair,
                quantity_raw: 1_000_000_000,
                entry_price: 100.0,
                current_price: 110.0,
                allocated_usd: 10.0,
                opened_at: Utc::now(),
            },
        );

        let snapshot = snapshot_with_price(token_pair, 110.0);
        let mut events = engine.subscribe();
        engine.evaluate_stop_losses(&snapshot).await;

        let mut saw_take_profit_skip = false;
        while let Ok(event) = events.try_recv() {
            if let LiveEvent::SignalSkipped { reason, .. } = event {
                if reason.contains("take profit") {
                    saw_take_profit_skip = true;
                }
            }
        }
        assert!(
            saw_take_profit_skip,
            "expected a take-profit close attempt while disabled"
        );
    }

    #[test]
    fn test_set_cross_dex_arb_enabled_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert!(!engine.status().cross_dex_arb_enabled);

        engine.set_cross_dex_arb_enabled(true);
        assert!(engine.status().cross_dex_arb_enabled);
    }

    #[test]
    fn test_set_cross_dex_min_spread_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert_eq!(engine.status().cross_dex_min_spread, 0.015);

        engine.set_cross_dex_min_spread(0.03);
        assert_eq!(engine.status().cross_dex_min_spread, 0.03);
    }

    #[test]
    fn test_set_cross_dex_max_slippage_bps_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert_eq!(engine.status().cross_dex_max_slippage_bps, 30);

        engine.set_cross_dex_max_slippage_bps(20);
        assert_eq!(engine.status().cross_dex_max_slippage_bps, 20);
    }

    #[test]
    fn test_set_cross_dex_min_net_edge_bps_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert_eq!(engine.status().cross_dex_min_net_edge_bps, 30);

        engine.set_cross_dex_min_net_edge_bps(25);
        assert_eq!(engine.status().cross_dex_min_net_edge_bps, 25);
    }

    #[test]
    fn test_set_pair_enabled_updates_status() {
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        assert!(engine.status().pairs.iter().all(|p| p.enabled));

        engine.set_pair_enabled(pair.label.to_string(), false);
        let status = engine.status();
        let entry = status
            .pairs
            .iter()
            .find(|p| p.pair_label == pair.label)
            .expect("configured pair must appear in status");
        assert!(!entry.enabled);

        engine.set_pair_enabled(pair.label.to_string(), true);
        assert!(engine.status().pairs.iter().all(|p| p.enabled));
    }

    #[test]
    fn test_required_cross_dex_spread_uses_operator_floor_when_higher() {
        // min_spread (1.5%) exceeds the cost-aware floor (30bps slippage
        // * 2 + 10bps margin = 70bps), so the operator's tighter
        // requirement wins.
        assert_eq!(required_cross_dex_spread(0.015, 30, 10), 0.015);
    }

    #[test]
    fn test_required_cross_dex_spread_uses_cost_floor_when_higher() {
        // A loose slippage tolerance (70bps/leg) with a low min_spread
        // (0.1%) must not let a trade through on a spread that a single
        // leg's slippage alone could erase -- the cost-aware floor
        // (2 * 70bps + 10bps = 150bps) wins instead.
        assert_eq!(required_cross_dex_spread(0.001, 70, 10), 0.015);
    }

    #[tokio::test]
    async fn test_evaluate_cross_dex_arbitrage_noop_when_disabled() {
        // Disabled by default (`cross_dex_arb_enabled: false` in
        // `test_config`) -- must return without attempting any network
        // I/O, since `test_engine` points at an unreachable RPC endpoint
        // and would hang/error if this actually tried to quote.
        let pair = test_pair();
        let engine = test_engine(pair, 50.0);
        let mut events = engine.subscribe();

        engine.evaluate_cross_dex_arbitrage().await;

        assert!(events.try_recv().is_err(), "expected no events at all");
    }

    #[tokio::test]
    async fn test_evaluate_cross_dex_arbitrage_retries_closing_open_position() {
        // A pair with an already-open position must attempt to flatten
        // it back to quote (via `close_position`), not look for a new
        // buy opportunity on top of it -- capital should keep cycling
        // rather than sitting stuck. On a disabled engine, `close_position`
        // takes its own no-network "would close" path, so this is safe
        // to assert without touching the (unreachable) test RPC endpoint.
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);
        engine.set_cross_dex_arb_enabled(true);

        engine.positions.lock().unwrap().insert(
            token_pair,
            LivePosition {
                id: PositionId::new(),
                pair: token_pair,
                quantity_raw: 1_000_000_000,
                entry_price: 100.0,
                current_price: 100.0,
                allocated_usd: 10.0,
                opened_at: Utc::now(),
            },
        );

        let mut events = engine.subscribe();
        engine.evaluate_cross_dex_arbitrage().await;

        let mut saw_close_attempt = false;
        while let Ok(event) = events.try_recv() {
            if let LiveEvent::SignalSkipped { reason, .. } = event {
                if reason.contains("cross-dex arb") {
                    saw_close_attempt = true;
                }
            }
        }
        assert!(
            saw_close_attempt,
            "expected an attempt to close the open position back to quote"
        );
    }

    #[tokio::test]
    async fn test_reconcile_untracked_balance_is_noop_when_position_already_tracked() {
        // Must not touch the network (and thus the unreachable test RPC
        // endpoint) once a position already exists for the pair -- the
        // whole point is to recover *gaps*, never double-count.
        let pair = test_pair();
        let token_pair = TokenPair::new(pair.base_mint, pair.quote_mint);
        let engine = test_engine(pair, 50.0);

        engine.positions.lock().unwrap().insert(
            token_pair,
            LivePosition {
                id: PositionId::new(),
                pair: token_pair,
                quantity_raw: 1_000_000_000,
                entry_price: 100.0,
                current_price: 100.0,
                allocated_usd: 10.0,
                opened_at: Utc::now(),
            },
        );

        let mut events = engine.subscribe();
        engine.reconcile_untracked_balance(&pair).await;

        assert!(
            events.try_recv().is_err(),
            "expected no events when a position is already tracked"
        );
        assert_eq!(engine.positions.lock().unwrap().len(), 1);
    }
}
