# Solstice Strategy Framework

**Purpose**: Define plugin-based strategy framework and strategy lifecycle management.

**Scope**: Strategy trait, plugin loading, signal generation, and strategy coordination.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Overview

Solstice uses a plugin-based strategy framework allowing multiple strategies to run concurrently without modifying the core platform. Strategies are Rust crates that implement a common interface.

**Key Characteristics**:
- **Pluggable**: Strategies are dynamically loaded plugins
- **Type-Safe**: Strategies use Rust, fully type-checked
- **Isolated**: Strategies cannot interfere with each other
- **Observable**: All strategy actions logged and metriced
- **Composable**: Multiple strategies can coordinate
- **Testable**: Strategies easily tested in isolation

---

## Strategy Trait

### Core Interface

```rust
pub trait Strategy: Send + Sync {
    /// Strategy name and version
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    
    /// Strategy configuration validation
    fn validate_config(&self, config: &serde_json::Value) -> Result<()>;
    
    /// Main evaluation function
    /// Called when market data is updated
    async fn evaluate(&self, 
        market_snapshot: &MarketSnapshot,
        portfolio_state: &PortfolioState,
        config: &serde_json::Value,
    ) -> Result<Vec<Signal>>;
    
    /// Optional: Initialize strategy state
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }
    
    /// Optional: Clean up strategy state
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
    
    /// Get strategy metadata
    fn metadata(&self) -> StrategyMetadata {
        StrategyMetadata {
            name: self.name().to_string(),
            version: self.version().to_string(),
            author: "".to_string(),
            description: "".to_string(),
            capabilities: vec![],
        }
    }
}

pub struct StrategyMetadata {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub capabilities: Vec<String>,  // e.g., ["spot_trading", "perpetuals"]
}
```

### Signal Type

```rust
pub struct Signal {
    pub strategy: String,           // Strategy name that generated
    pub signal_id: String,          // Unique signal ID
    pub timestamp: DateTime<Utc>,
    pub signal_type: SignalType,
    pub confidence: f64,            // 0.0 to 1.0
    pub suggested_size: Option<u64>,  // Suggested position size
    pub metadata: serde_json::Value, // Strategy-specific data
}

pub enum SignalType {
    Buy(Asset),                    // Long signal
    Sell(Asset),                   // Short signal / Exit
    Close(PositionId),             // Close specific position
    Rebalance { reason: String },  // Rebalance portfolio
}

pub struct Asset {
    pub mint: Pubkey,
    pub quote: Pubkey,             // Quote currency (usually USDC)
    pub fair_value: Price,
    pub suggested_entry: Price,
    pub stop_loss: Option<Price>,
}
```

---

## Strategy Lifecycle

### Initialization

```rust
pub struct StrategyManager {
    strategies: Arc<RwLock<HashMap<String, Arc<dyn Strategy>>>>,
    signal_tx: Broadcaster<Signal>,
    config: Arc<StrategyConfig>,
}

impl StrategyManager {
    pub async fn new(config: &StrategyConfig) -> Result<Self> {
        Ok(Self {
            strategies: Arc::new(RwLock::new(HashMap::new())),
            signal_tx: Broadcaster::new(10000),
            config: Arc::new(config.clone()),
        })
    }
    
    pub async fn load_strategy(&self, path: &Path) -> Result<()> {
        // Dynamic library loading (using libloading crate)
        let lib = libloading::Library::new(path)?;
        
        unsafe {
            let constructor: libloading::Symbol<extern "C" fn() -> *mut dyn Strategy>
                = lib.get(b"create_strategy")?;
            
            let strategy = Box::from_raw(constructor());
            
            // Validate strategy
            strategy.validate_config(&self.config.strategy_config)?;
            
            // Initialize strategy
            strategy.initialize().await?;
            
            // Register strategy
            let strategies = &mut *self.strategies.write().await;
            strategies.insert(
                strategy.name().to_string(),
                Arc::from(strategy),
            );
        }
        
        Ok(())
    }
    
    pub async fn unload_strategy(&self, name: &str) -> Result<()> {
        let strategies = &mut *self.strategies.write().await;
        
        if let Some(strategy) = strategies.remove(name) {
            strategy.shutdown().await?;
        }
        
        Ok(())
    }
}
```

---

## Market Snapshot

Data provided to strategies for evaluation:

```rust
pub struct MarketSnapshot {
    pub timestamp: DateTime<Utc>,
    pub slot: u64,
    pub prices: HashMap<Pubkey, Price>,
    pub orderbooks: HashMap<Pubkey, OrderBook>,
    pub liquidity: HashMap<Pubkey, Liquidity>,
    pub volumes_24h: HashMap<Pubkey, u64>,
}

pub struct Price {
    pub token: Pubkey,
    pub quote: Pubkey,
    pub price: f64,
    pub confidence: f64,
    pub sources: Vec<PriceSource>,
}

pub enum PriceSource {
    Yellowstone,
    RpcPolling,
    DexApi,
    Oracle,
}

pub struct OrderBook {
    pub market: Pubkey,
    pub bids: Vec<(Price, Quantity)>,
    pub asks: Vec<(Price, Quantity)>,
    pub spread: Price,
    pub mid_price: Price,
    pub timestamp: DateTime<Utc>,
}

pub struct Liquidity {
    pub pool: Pubkey,
    pub token_a: Pubkey,
    pub token_b: Pubkey,
    pub reserve_a: u64,
    pub reserve_b: u64,
    pub utilization: f64,      // 0.0 to 1.0
}
```

### Portfolio State

```rust
pub struct PortfolioState {
    pub timestamp: DateTime<Utc>,
    pub positions: Vec<Position>,
    pub total_value_usd: f64,
    pub available_capital: u64,
    pub risk_metrics: RiskMetrics,
}

pub struct Position {
    pub id: PositionId,
    pub mint: Pubkey,
    pub quantity: i64,          // Negative for shorts (when allowed)
    pub entry_price: Price,
    pub current_price: Price,
    pub unrealized_pnl: f64,
    pub age: Duration,
    pub fees_paid: f64,
}

pub struct RiskMetrics {
    pub max_drawdown: f64,
    pub daily_pnl: f64,
    pub daily_loss: f64,
    pub exposure_percent: f64,
    pub concentration: HashMap<Pubkey, f64>,  // % per asset
}
```

---

## Strategy Evaluation

### Evaluation Flow

```
Market Data Update
       ↓
┌──────────────────────────────┐
│ Market Snapshot Created      │
└──────────┬───────────────────┘
           ↓
┌──────────────────────────────┐
│ For Each Active Strategy:    │
│ 1. Update portfolio state    │
│ 2. Call evaluate()           │
│ 3. Validate signals          │
└──────────┬───────────────────┘
           ↓
┌──────────────────────────────┐
│ Deduplicate Signals          │
│ (Remove duplicates/conflicts)│
└──────────┬───────────────────┘
           ↓
┌──────────────────────────────┐
│ Rank Signals by Confidence   │
└──────────┬───────────────────┘
           ↓
  Emit to Execution Engine
```

### Concurrent Evaluation

```rust
impl StrategyManager {
    pub async fn evaluate_all(&self, 
        snapshot: &MarketSnapshot,
        portfolio: &PortfolioState,
    ) -> Result<Vec<Signal>> {
        let strategies = self.strategies.read().await;
        
        // Evaluate all strategies concurrently
        let futures: Vec<_> = strategies
            .values()
            .map(|strategy| {
                let strategy = strategy.clone();
                let snapshot = snapshot.clone();
                let portfolio = portfolio.clone();
                let config = self.config.strategy_config.clone();
                
                async move {
                    match strategy.evaluate(&snapshot, &portfolio, &config).await {
                        Ok(signals) => Some(signals),
                        Err(e) => {
                            error!("Strategy {} evaluation failed: {}", 
                                   strategy.name(), e);
                            None
                        }
                    }
                }
            })
            .collect();
        
        let results = futures::future::join_all(futures).await;
        
        let all_signals: Vec<Signal> = results
            .into_iter()
            .filter_map(|r| r)
            .flatten()
            .collect();
        
        // Deduplicate and rank signals
        Ok(self.rank_signals(all_signals).await)
    }
}
```

---

## Signal Validation & Filtering

### Validation Rules

```rust
pub struct SignalValidator;

impl SignalValidator {
    pub fn validate(&self, signal: &Signal) -> Result<()> {
        // Confidence between 0 and 1
        if signal.confidence < 0.0 || signal.confidence > 1.0 {
            return Err("Invalid confidence".into());
        }
        
        // Confidence minimum
        if signal.confidence < 0.5 {
            return Err("Confidence too low".into());
        }
        
        // Suggested size positive
        if let Some(size) = signal.suggested_size {
            if size == 0 {
                return Err("Zero suggested size".into());
            }
        }
        
        Ok(())
    }
}
```

### Deduplication

```rust
pub struct SignalDeduplicator {
    recent_signals: Arc<Mutex<VecDeque<Signal>>>,
    ttl: Duration,
}

impl SignalDeduplicator {
    pub async fn deduplicate(&self, signals: Vec<Signal>) -> Vec<Signal> {
        let mut recent = self.recent_signals.lock().await;
        
        // Remove stale signals
        let now = Utc::now();
        recent.retain(|s| now.signed_duration_since(s.timestamp) < self.ttl);
        
        // Filter new signals (keep only non-duplicate)
        signals.into_iter()
            .filter(|signal| {
                let is_duplicate = recent.iter()
                    .any(|s| s.signal_id == signal.signal_id);
                
                if !is_duplicate {
                    recent.push_back(signal.clone());
                }
                
                !is_duplicate
            })
            .collect()
    }
}
```

### Signal Ranking

```rust
pub struct SignalRanker;

impl SignalRanker {
    pub fn rank(signals: Vec<Signal>) -> Vec<Signal> {
        let mut ranked = signals;
        
        // Sort by confidence (highest first)
        ranked.sort_by(|a, b| {
            b.confidence.partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        
        ranked
    }
}
```

---

## Built-In Strategies

Solstice includes reference strategies demonstrating best practices:

### 1. Simple Moving Average Strategy

```rust
pub struct SimpleMovingAverageStrategy {
    short_period: usize,
    long_period: usize,
}

impl Strategy for SimpleMovingAverageStrategy {
    fn name(&self) -> &str { "SMA" }
    fn version(&self) -> &str { "1.0.0" }
    
    async fn evaluate(&self, 
        snapshot: &MarketSnapshot,
        portfolio: &PortfolioState,
        config: &serde_json::Value,
    ) -> Result<Vec<Signal>> {
        let short_sma = self.calculate_sma(snapshot, self.short_period)?;
        let long_sma = self.calculate_sma(snapshot, self.long_period)?;
        
        let mut signals = vec![];
        
        if short_sma > long_sma {
            signals.push(Signal {
                strategy: "SMA".to_string(),
                signal_type: SignalType::Buy(Asset {
                    mint: snapshot.primary_token,
                    quote: USDC,
                    fair_value: Price::from_snapshot(snapshot),
                    suggested_entry: Price::from_snapshot(snapshot),
                    stop_loss: None,
                }),
                confidence: 0.65,
                suggested_size: Some(10_000_000),
                metadata: json!({"short_sma": short_sma, "long_sma": long_sma}),
            });
        }
        
        Ok(signals)
    }
}
```

### 2. Spread Arbitrage Strategy

```rust
pub struct SpreadArbitrageStrategy;

impl Strategy for SpreadArbitrageStrategy {
    fn name(&self) -> &str { "SpreadArb" }
    fn version(&self) -> &str { "1.0.0" }
    
    async fn evaluate(&self, 
        snapshot: &MarketSnapshot,
        portfolio: &PortfolioState,
        config: &serde_json::Value,
    ) -> Result<Vec<Signal>> {
        let mut signals = vec![];
        
        // Look for assets trading at different prices on different DEXes
        for (market1, price1) in &snapshot.prices {
            for (market2, price2) in &snapshot.prices {
                if market1 == market2 {
                    continue;
                }
                
                let spread = (price2.price - price1.price).abs() / price1.price;
                
                if spread > 0.02 {  // 2% spread
                    // Generate arbitrage signal
                    let signal = Signal {
                        strategy: "SpreadArb".to_string(),
                        signal_type: SignalType::Buy(Asset {
                            mint: *market1,
                            quote: USDC,
                            fair_value: Price::average(price1, price2),
                            suggested_entry: *price1,
                            stop_loss: None,
                        }),
                        confidence: 0.8,
                        suggested_size: Some(5_000_000),
                        metadata: json!({"spread": spread, "market2": market2}),
                    };
                    
                    signals.push(signal);
                }
            }
        }
        
        Ok(signals)
    }
}
```

---

## Configuration

```toml
[strategy]
# Framework
plugin_directory = "./strategies"
max_concurrent_strategies = 5
evaluation_interval_ms = 100
signal_batch_size = 100

# Default parameters
[strategy.defaults]
min_confidence = 0.65
max_slippage_percent = 1.5
min_spread_basis_points = 5
position_decay_hours = 24

# Signal deduplication
[strategy.deduplication]
ttl_seconds = 60
enabled = true

# Per-strategy configuration
[strategy.strategies]
sma = { enabled = true, short_period = 20, long_period = 50 }
spread_arb = { enabled = true, min_spread_bps = 20 }
```

---

## Testing

```rust
#[test]
fn test_strategy_trait() {
    // Create mock strategy
    let strategy = MockStrategy::new();
    
    // Verify trait implementation
    assert_eq!(strategy.name(), "Mock");
    assert!(!strategy.version().is_empty());
}

#[tokio::test]
async fn test_signal_generation() {
    let strategy = SimpleMovingAverageStrategy {
        short_period: 10,
        long_period: 20,
    };
    
    let snapshot = create_test_snapshot();
    let portfolio = create_test_portfolio();
    let config = json!({});
    
    let signals = strategy.evaluate(&snapshot, &portfolio, &config).await.unwrap();
    
    assert!(!signals.is_empty());
    assert!(signals[0].confidence >= 0.5 && signals[0].confidence <= 1.0);
}

#[tokio::test]
async fn test_strategy_manager() {
    let manager = StrategyManager::new(&test_config()).await.unwrap();
    
    // Load strategy
    manager.load_strategy(Path::new("./target/debug/libstrategy_sma.so")).await.unwrap();
    
    // Verify loaded
    let strategies = manager.strategies.read().await;
    assert!(strategies.contains_key("SMA"));
}
```

---

## Related Documents

- [STAT_ARBS.md](./STAT_ARBS.md) - Statistical arbitrage implementation
- [FAIR_VALUE.md](./FAIR_VALUE.md) - Fair value computation
- [EXECUTION.md](./EXECUTION.md) - Signal execution
- [TESTING_STRATEGY.md](./TESTING_STRATEGY.md) - Testing framework
- [ARCHITECTURE.md](./ARCHITECTURE.md) - System architecture

---

**Next**: [STAT_ARBS.md](./STAT_ARBS.md) - Statistical arbitrage engine

Specification progress: 12/45 documents complete (27%)
