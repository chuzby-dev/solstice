# Solstice Rust Workspace Architecture

**Purpose**: Define the Rust workspace structure, crate organization, responsibilities, and interfaces.

**Scope**: Workspace layout, crate-level design, public APIs, and inter-crate dependencies.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Overview

Solstice is organized as a Rust workspace with specialized crates, each with a single, well-defined responsibility. The workspace enables:

- Isolated compilation and testing
- Clear dependency boundaries
- Parallel development
- Shared core abstractions
- Type-safe inter-crate communication

---

## Workspace Structure

```
solstice/
├── Cargo.toml                    # Workspace root
├── Cargo.lock                    # Locked dependencies
│
├── crates/
│   ├── solstice-core/            # Core types, traits, abstractions
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── event.rs
│   │       ├── price.rs
│   │       ├── position.rs
│   │       ├── signal.rs
│   │       └── ...
│   │
│   ├── solstice-market-data/     # Market data ingestion
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── ingestion.rs
│   │       ├── normalizer.rs
│   │       ├── cache.rs
│   │       └── ...
│   │
│   ├── solstice-blockchain/      # Solana & blockchain integration
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── rpc.rs
│   │       ├── yellowstone.rs
│   │       ├── transaction.rs
│   │       └── ...
│   │
│   ├── solstice-dex/             # DEX protocol implementations
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── jupiter.rs
│   │       ├── raydium.rs
│   │       ├── orca.rs
│   │       ├── meteora.rs
│   │       ├── phoenix.rs
│   │       ├── openbook.rs
│   │       └── ...
│   │
│   ├── solstice-strategy/        # Strategy framework & engines
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── framework.rs
│   │       ├── plugin.rs
│   │       ├── stat_arbs.rs
│   │       ├── fair_value.rs
│   │       ├── signal.rs
│   │       └── ...
│   │
│   ├── solstice-execution/       # Execution & risk management
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── executor.rs
│   │       ├── risk.rs
│   │       ├── position_sizing.rs
│   │       ├── builder.rs
│   │       └── ...
│   │
│   ├── solstice-storage/         # Data persistence
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── postgres.rs
│   │       ├── redis.rs
│   │       ├── schema.rs
│   │       └── ...
│   │
│   ├── solstice-api/             # REST & WebSocket APIs
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── http.rs
│   │       ├── websocket.rs
│   │       ├── handlers.rs
│   │       └── ...
│   │
│   ├── solstice-simulation/      # Backtesting & paper trading
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── engine.rs
│   │       ├── replay.rs
│   │       ├── metrics.rs
│   │       └── ...
│   │
│   └── solstice-cli/             # Command-line interface
│       └── src/
│           ├── main.rs
│           ├── commands/
│           ├── config.rs
│           └── ...
│
├── dashboard/                    # React TypeScript frontend
│   ├── package.json
│   ├── src/
│   │   ├── components/
│   │   ├── pages/
│   │   ├── hooks/
│   │   ├── services/
│   │   └── ...
│   └── ...
│
└── docs/                         # Technical specification
    ├── ARCHITECTURE.md
    ├── WORKSPACE.md
    └── ...
```

---

## Core Crate Responsibilities

### solstice-core

**Purpose**: Shared types, traits, and abstractions used by all crates.

**Responsibilities**:
- Core data types (Price, Position, Signal, Order, etc.)
- Trait definitions and abstractions
- Error types and error handling
- Time utilities and types
- Configuration structures
- Logging infrastructure

**Key Types**:
```rust
pub struct Price { /* ... */ }
pub struct Position { /* ... */ }
pub struct Signal { /* ... */ }
pub struct Trade { /* ... */ }
pub struct OrderBook { /* ... */ }
pub enum TradingEvent { /* ... */ }
pub trait MarketDataSource { /* ... */ }
pub trait Strategy { /* ... */ }
pub trait RiskManager { /* ... */ }
```

**Public API**: Exports all core types and traits; minimal logic.

**Dependencies**: 
- Standard library
- `serde`, `tokio`, `tracing`
- Minimal external dependencies

**Not Responsible For**:
- Implementation logic
- External integrations
- Database schema
- API definitions

---

### solstice-market-data

**Purpose**: Ingests, normalizes, and caches market data from multiple sources.

**Responsibilities**:
- Connect to market data sources (Yellowstone, RPC, DEX APIs)
- Normalize diverse data formats into common event types
- Cache market data for strategy consumption
- Deduplicate and validate market events
- Persist market snapshots
- Handle backpressure and retry logic

**Key Components**:

1. **Source Adapters**: Connect to specific data sources
   - `YellowstoneAdapter` - Yellowstone gRPC account stream
   - `RpcAdapter` - Solana RPC queries
   - `DexAdapter` - Direct DEX protocol queries

2. **Normalizer**: Converts source-specific formats to common events

3. **Cache**: In-memory cache of recent market data
   - OrderBooks
   - Token prices
   - Account state snapshots

4. **Deduplicator**: Removes duplicate/stale events

5. **Persister**: Writes market snapshots to storage

**Public API**:
```rust
pub trait DataSource {
    async fn subscribe(&self, tokens: &[Pubkey]) -> Receiver<MarketEvent>;
    async fn query_orderbook(&self, market: Pubkey) -> Result<OrderBook>;
    async fn query_price(&self, token: Pubkey) -> Result<Price>;
}

pub struct MarketDataManager { /* ... */ }
impl MarketDataManager {
    pub async fn new(config: &Config) -> Result<Self>;
    pub async fn subscribe(&self) -> Receiver<MarketEvent>;
    pub async fn query_orderbook(&self, market: Pubkey) -> Result<OrderBook>;
}
```

**Dependencies**:
- `solstice-core`
- `tokio`, `tonic` (gRPC)
- `jsonrpc-client` (RPC)
- `tracing`

**See Also**: [MARKET_DATA.md](./MARKET_DATA.md), [YELLOWSTONE.md](./YELLOWSTONE.md)

---

### solstice-blockchain

**Purpose**: Abstracts interaction with Solana blockchain and provides RPC utilities.

**Responsibilities**:
- Connect to Solana RPC nodes
- Query account state and transaction history
- Build and sign transactions
- Monitor transaction confirmation
- Abstract RPC failover and retry logic
- Provide typed access to program state
- Fee estimation and cost tracking

**Key Components**:

1. **RPC Client**: Unified interface to Solana RPC
   - Connection pooling
   - Automatic failover
   - Rate limiting and backpressure

2. **Account State**: Typed access to account data
   - Token mints
   - Market state
   - Pool state (for DEX protocols)

3. **Transaction Builder**: Low-level transaction construction

4. **Confirmation Monitor**: Tracks transaction confirmation status

5. **Fee Estimator**: Estimates transaction costs

**Public API**:
```rust
pub struct BlockchainClient { /* ... */ }
impl BlockchainClient {
    pub async fn new(config: &BlockchainConfig) -> Result<Self>;
    pub async fn get_account(&self, address: &Pubkey) -> Result<Account>;
    pub async fn get_token_supply(&self, mint: &Pubkey) -> Result<u64>;
    pub async fn simulate_transaction(&self, tx: &Transaction) -> Result<SimulationResult>;
    pub async fn send_transaction(&self, tx: &Transaction) -> Result<Signature>;
    pub async fn confirm_transaction(&self, sig: &Signature) -> Result<Status>;
}

pub struct TransactionBuilder { /* ... */ }
impl TransactionBuilder {
    pub fn new() -> Self;
    pub fn add_instruction(&mut self, ix: Instruction) -> &mut Self;
    pub fn build(&self, payer: &Pubkey) -> Result<Transaction>;
}
```

**Dependencies**:
- `solstice-core`
- `solana-sdk`, `solana-client`
- `tokio`, `tonic`
- `tracing`

**See Also**: [SOLANA_RPC.md](./SOLANA_RPC.md)

---

### solstice-dex

**Purpose**: Provides unified interface to DEX protocols and quote engines.

**Responsibilities**:
- Abstract each DEX protocol (Jupiter, Raydium, Orca, etc.)
- Query orderbooks and available liquidity
- Get price quotes for swaps
- Identify best execution route across multiple DEXes
- Calculate routing for complex multi-leg trades
- Track liquidity pools and their parameters

**Key Components**:

1. **DEX Protocols**: Individual implementations
   - `JupiterDex` - Jupiter aggregator
   - `RaydiumDex` - Raydium AMM
   - `OrcaDex` - Orca AMM
   - `MeteoraDex` - Meteora AMM
   - `PhoenixDex` - Phoenix CLOB
   - `OpenBookDex` - OpenBook CLOB

2. **Unified Interface**: Common trait for all DEXes
   - Get quotes
   - Build swap instructions
   - Query available liquidity

3. **Route Finder**: Determines optimal execution path
   - Multi-leg route optimization
   - Cross-DEX aggregation
   - Slippage minimization

4. **Liquidity Monitor**: Tracks pool state and changes

**Public API**:
```rust
pub trait DexClient {
    async fn get_quote(&self, swap: &SwapRequest) -> Result<Quote>;
    async fn build_swap(&self, quote: &Quote) -> Result<SwapInstructions>;
    async fn get_liquidity(&self, pair: &TokenPair) -> Result<Liquidity>;
}

pub struct DexAggregator { /* ... */ }
impl DexAggregator {
    pub async fn get_best_route(&self, swap: &SwapRequest) -> Result<Route>;
    pub async fn estimate_slippage(&self, swap: &SwapRequest) -> Result<f64>;
}
```

**Dependencies**:
- `solstice-core`, `solstice-blockchain`
- `tokio`, `tonic`
- `tracing`

**See Also**: [DEX_INTEGRATIONS.md](./DEX_INTEGRATIONS.md)

---

### solstice-strategy

**Purpose**: Implements strategy framework and core trading strategies.

**Responsibilities**:
- Define strategy plugin interface
- Implement statistical arbitrage engine
- Implement fair value computation
- Implement signal generation pipeline
- Implement portfolio management logic
- Load and manage strategy instances
- Coordinate multi-strategy execution

**Key Components**:

1. **Strategy Plugin System**:
   - Trait for pluggable strategies
   - Strategy registry and loader
   - Strategy lifecycle management

2. **Statistical Arbitrage Engine**:
   - Identify mispricing opportunities
   - Correlation analysis
   - Cointegration detection
   - Mean reversion detection

3. **Fair Value Engine**:
   - Compute intrinsic prices
   - Multi-source price weighting
   - Time-decay adjustments

4. **Signal Generator**:
   - Evaluate trading signals
   - Multi-factor scoring
   - Confidence calculation

5. **Portfolio Manager**:
   - Track open positions
   - Calculate rebalancing needs
   - Manage position concentration

**Public API**:
```rust
pub trait Strategy: Send + Sync {
    async fn evaluate(&self, market: &MarketSnapshot) -> Result<Vec<Signal>>;
    async fn update_position(&self, update: &PositionUpdate);
    fn name(&self) -> &str;
    fn version(&self) -> &str;
}

pub struct StrategyFramework { /* ... */ }
impl StrategyFramework {
    pub async fn new(config: &Config) -> Result<Self>;
    pub async fn register_strategy(&mut self, strategy: Box<dyn Strategy>);
    pub async fn evaluate_all(&self, market: &MarketSnapshot) -> Result<Vec<Signal>>;
}

pub struct StatArbEngine { /* ... */ }
impl StatArbEngine {
    pub async fn find_opportunities(&self, market: &MarketSnapshot) -> Result<Vec<Opportunity>>;
}

pub struct FairValueEngine { /* ... */ }
impl FairValueEngine {
    pub async fn compute_fair_value(&self, token: &Pubkey, prices: &[Price]) -> Result<Price>;
}
```

**Dependencies**:
- `solstice-core`, `solstice-market-data`
- `tokio`
- `ndarray`, `ndarray-stats` (numerical computing)
- `tracing`

**See Also**: [STRATEGY_FRAMEWORK.md](./STRATEGY_FRAMEWORK.md), [STAT_ARBS.md](./STAT_ARBS.md)

---

### solstice-execution

**Purpose**: Plans execution and enforces risk management.

**Responsibilities**:
- Calculate position sizes based on risk parameters
- Plan execution strategy (partial fills, routing, timing)
- Enforce risk limits and hard stops
- Build optimized transactions
- Manage order lifecycle
- Track and record fills
- Calculate P&L and Greeks

**Key Components**:

1. **Position Sizer**:
   - Calculate trade quantities
   - Respect risk budgets
   - Enforce concentration limits

2. **Risk Manager**:
   - Enforce hard position limits
   - Stop-loss management
   - Loss limit enforcement
   - Exposure limits

3. **Execution Planner**:
   - Determine execution path
   - Optimize for slippage vs. impact
   - Schedule execution (immediate vs. scheduled)

4. **Transaction Builder**:
   - Construct optimal transaction
   - Route through DEXes
   - Optimize fees

5. **Order Manager**:
   - Track pending orders
   - Monitor fills
   - Handle partial execution

**Public API**:
```rust
pub struct PositionSizer { /* ... */ }
impl PositionSizer {
    pub fn calculate_size(&self, signal: &Signal, params: &RiskParams) -> Result<Quantity>;
}

pub struct RiskManager { /* ... */ }
impl RiskManager {
    pub fn can_trade(&self, signal: &Signal, size: Quantity) -> Result<()>;
    pub fn update_limits(&mut self, limits: &RiskLimits);
}

pub struct ExecutionPlanner { /* ... */ }
impl ExecutionPlanner {
    pub async fn plan(&self, signal: &Signal, size: Quantity) -> Result<ExecutionPlan>;
}

pub struct OrderManager { /* ... */ }
impl OrderManager {
    pub async fn submit_order(&mut self, plan: ExecutionPlan) -> Result<OrderId>;
    pub async fn monitor_order(&self, id: OrderId) -> Receiver<OrderEvent>;
}
```

**Dependencies**:
- `solstice-core`, `solstice-dex`, `solstice-blockchain`
- `tokio`
- `tracing`

**See Also**: [EXECUTION.md](./EXECUTION.md), [RISK_MANAGEMENT.md](./RISK_MANAGEMENT.md)

---

### solstice-storage

**Purpose**: Persists all platform data and provides query interface.

**Responsibilities**:
- Connect to PostgreSQL + TimescaleDB
- Connect to Redis cache
- Define and maintain database schema
- Provide typed query interface
- Handle connection pooling
- Implement caching strategy
- Manage data retention policies

**Key Components**:

1. **PostgreSQL Connection**:
   - Connection pooling
   - Migration management
   - Query builder interface

2. **Redis Connection**:
   - Connection pooling
   - Pub/sub interface
   - Cache management

3. **Schema Definitions**:
   - Market data tables
   - Trade history
   - Position history
   - Account state snapshots

4. **Query Layer**:
   - Type-safe queries
   - Time-range filters
   - Aggregation queries

5. **Cache Manager**:
   - Cache invalidation
   - Cache warming
   - TTL management

**Public API**:
```rust
pub struct StoragePool { /* ... */ }
impl StoragePool {
    pub async fn new(config: &StorageConfig) -> Result<Self>;
    pub async fn get_market_data(&self, token: &Pubkey, range: TimeRange) -> Result<Vec<MarketSnapshot>>;
    pub async fn save_trade(&self, trade: &Trade) -> Result<()>;
    pub async fn get_position_history(&self, id: PositionId) -> Result<Vec<PositionUpdate>>;
}

pub struct CacheManager { /* ... */ }
impl CacheManager {
    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    pub async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> Result<()>;
}
```

**Dependencies**:
- `solstice-core`
- `tokio`, `tokio-postgres`
- `redis`
- `sqlx` (compile-time SQL verification)
- `tracing`

**See Also**: [DATABASE.md](./DATABASE.md), [REDIS_ARCHITECTURE.md](./REDIS_ARCHITECTURE.md)

---

### solstice-api

**Purpose**: Exposes platform capabilities via REST and WebSocket APIs.

**Responsibilities**:
- Serve REST API endpoints
- Manage WebSocket connections
- Handle authentication and rate limiting
- Serialize/deserialize API messages
- Provide real-time updates via WebSocket
- Implement OpenAPI spec

**Key Components**:

1. **HTTP Server** (Axum):
   - Route handlers
   - Request/response serialization
   - Error handling
   - Rate limiting middleware

2. **WebSocket Server**:
   - Connection management
   - Subscription handling
   - Real-time event broadcasting

3. **Handlers**:
   - Configuration endpoints
   - Status endpoints
   - Trading endpoints
   - Historical query endpoints

4. **Authentication**:
   - API key validation
   - Request signing
   - Access control

**Public API**:
```rust
pub struct ApiServer { /* ... */ }
impl ApiServer {
    pub async fn new(config: &ApiConfig) -> Result<Self>;
    pub async fn start(&self) -> Result<()>;
}

// REST endpoints
GET /api/v1/status
GET /api/v1/positions
GET /api/v1/positions/{id}
GET /api/v1/trades
GET /api/v1/performance
POST /api/v1/trading/{action}
GET /api/v1/markets/{token}
WebSocket /api/v1/ws
```

**Dependencies**:
- `solstice-core`, `solstice-storage`, `solstice-execution`
- `axum`, `tokio-tungstenite`
- `serde`, `serde_json`
- `tracing`

**See Also**: [REST_API.md](./REST_API.md), [WEBSOCKET_API.md](./WEBSOCKET_API.md)

---

### solstice-simulation

**Purpose**: Backtesting, paper trading, and performance simulation.

**Responsibilities**:
- Replay historical market data
- Simulate strategy execution
- Calculate performance metrics
- Collect statistics and analytics
- Validate strategy logic before live trading
- Measure latency and slippage

**Key Components**:

1. **Simulation Engine**:
   - Time-based event loop
   - Market data replay
   - Strategy evaluation simulation

2. **Market Replay**:
   - Load historical data
   - Replay events in order
   - Handle time progression

3. **Order Simulator**:
   - Simulate order execution
   - Apply realistic slippage
   - Model partial fills

4. **Performance Calculator**:
   - Calculate returns
   - Calculate Sharpe ratio, max drawdown
   - Generate performance reports

5. **Paper Trading Mode**:
   - Live data with simulated execution
   - Real-time metrics
   - Seamless transition to live trading

**Public API**:
```rust
pub struct SimulationEngine { /* ... */ }
impl SimulationEngine {
    pub async fn new(config: &SimulationConfig) -> Result<Self>;
    pub async fn run_backtest(&self, start: DateTime, end: DateTime) -> Result<BacktestResults>;
    pub async fn run_paper_trading(&self) -> Result<PaperTradeSession>;
}

pub struct BacktestResults {
    pub total_return: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown: f64,
    pub trades: Vec<TradeRecord>,
}
```

**Dependencies**:
- `solstice-core`, `solstice-strategy`, `solstice-execution`, `solstice-storage`
- `tokio`
- `tracing`

**See Also**: [BACKTESTING.md](./BACKTESTING.md), [PAPER_TRADING.md](./PAPER_TRADING.md)

---

### solstice-cli

**Purpose**: Command-line interface for platform operation and administration.

**Responsibilities**:
- Start/stop platform services
- Configure platform settings
- Load strategies
- Execute backtest runs
- Query historical data
- Monitor running platform
- Administrative tasks

**Key Commands**:
```
solstice start              # Start platform in live trading mode
solstice backtest [opts]    # Run backtest
solstice paper-trade [opts] # Run paper trading
solstice config             # Show/edit configuration
solstice strategies list    # List available strategies
solstice strategies load    # Load strategy plugin
solstice query              # Query historical data
solstice monitor            # Monitor running platform
```

**Public API**:
```rust
// Implemented as binary crate, not library
fn main() -> Result<()> { /* ... */ }
```

**Dependencies**:
- `solstice-core`, `solstice-*` (all other crates)
- `clap` (CLI argument parsing)
- `tokio`
- `tracing`, `tracing-subscriber`

**See Also**: [CONFIGURATION.md](./CONFIGURATION.md), [OPERATIONAL_RUNBOOKS.md](./OPERATIONAL_RUNBOOKS.md)

---

## Inter-Crate Dependencies

```
solstice-core
  ↑
  ├── solstice-market-data
  ├── solstice-blockchain
  ├── solstice-dex
  ├── solstice-strategy
  ├── solstice-execution
  ├── solstice-storage
  ├── solstice-api
  ├── solstice-simulation
  └── solstice-cli (binary)

solstice-market-data
  ↑
  ├── solstice-strategy
  ├── solstice-simulation
  └── solstice-cli

solstice-blockchain
  ↑
  ├── solstice-dex
  ├── solstice-execution
  └── solstice-cli

solstice-dex
  ↑
  ├── solstice-execution
  └── solstice-cli

solstice-strategy
  ↑
  ├── solstice-execution
  ├── solstice-api
  ├── solstice-simulation
  └── solstice-cli

solstice-execution
  ↑
  ├── solstice-api
  ├── solstice-simulation
  └── solstice-cli

solstice-storage
  ↑
  ├── solstice-api
  ├── solstice-simulation
  └── solstice-cli

solstice-api
  ↑
  └── (no reverse dependencies)

solstice-simulation
  ↑
  └── solstice-cli

solstice-cli
  └── (no reverse dependencies; depends on all)
```

---

## Dependency Principles

1. **No Circular Dependencies**: Acyclic dependency graph
2. **Minimal Coupling**: Each crate imports only what it needs
3. **Interface Stability**: Public APIs rarely change
4. **Internal Flexibility**: Internal structure can change freely
5. **Version Independence**: Crates can be versioned independently (optional)

---

## Module Organization Within Crates

Each crate follows this pattern:

```rust
// lib.rs - Public API
pub mod public_module;
pub use public_module::PublicType;

mod internal;  // Private implementation
```

**Guidelines**:
- Prefix internal-only types with `_Internal` or put in `private` module
- Re-export important types in `lib.rs`
- Use `pub mod` only for public APIs
- Use `pub use` for type re-exports

---

## Testing Strategy

See [TESTING_STRATEGY.md](./TESTING_STRATEGY.md) for comprehensive testing approach.

Each crate includes:
- Unit tests (in same file as code)
- Integration tests (in `tests/` directory)
- Mock implementations for dependencies

---

## Feature Flags

Suggested workspace-level feature flags:

```toml
[features]
default = []
paper-trading = []
backtesting = []
jito-integration = []
metrics = []
```

Individual crates may have additional flags for optional functionality.

---

## Related Documents

- [ARCHITECTURE.md](./ARCHITECTURE.md) - System-level architecture
- [TESTING_STRATEGY.md](./TESTING_STRATEGY.md) - Testing approach per crate
- [CODING_STANDARDS.md](./CODING_STANDARDS.md) - Rust coding conventions
- [CI_CD.md](./CI_CD.md) - Build and test pipeline

---

**Next**: [DESIGN_RATIONALE.md](./DESIGN_RATIONALE.md)
