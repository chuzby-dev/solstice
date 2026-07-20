# Solstice Market Data Ingestion Architecture

**Purpose**: Define market data ingestion pipeline, normalization, and caching strategy.

**Scope**: Data sources, ingestion flow, event normalization, real-time processing, and backpressure handling.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Overview

The market data layer is the foundation of Solstice. It ingests prices and state changes from multiple sources, normalizes them into a common event stream, caches them for fast access, and feeds them to the strategy layer.

**Key Characteristics**:
- **Low Latency**: Sub-500ms latency from source to cache
- **High Throughput**: 10,000+ events/second capacity
- **Multiple Sources**: Yellowstone, RPC, DEX APIs
- **Normalized Events**: Common event format across sources
- **Deduplication**: Removes stale and duplicate events
- **Backpressure Handling**: Graceful degradation under load
- **Auditable**: All ingestion events logged

---

## Data Sources

### 1. Yellowstone gRPC (Primary)

**Purpose**: Real-time Solana account state changes

**Characteristics**:
- Atomic ordering from validator
- Low latency (50-200ms)
- Ordered by block slot
- Streaming architecture
- High-throughput capable

**Data Types**:
- Account state updates (market accounts, token mints)
- Token balance changes
- Oracle price updates
- Pool state changes

**Adapter Responsibility** (`YellowstoneAdapter`):
```rust
pub trait YellowstoneAdapter {
    async fn connect(&self, subscriptions: &[Pubkey]) -> Result<Receiver<RawAccountUpdate>>;
    async fn subscribe(&self, accounts: &[Pubkey]) -> Result<()>;
    async fn unsubscribe(&self, accounts: &[Pubkey]) -> Result<()>;
}
```

**Configuration**:
```toml
[market_data.yellowstone]
grpc_endpoints = ["grpcs://validator.solana.com:10900"]
max_subscriptions = 100
retry_enabled = true
```

See [YELLOWSTONE.md](./YELLOWSTONE.md) for detailed architecture.

---

### 2. Solana RPC (Fallback/Supplementary)

**Purpose**: State queries and fallback when Yellowstone unavailable

**Characteristics**:
- Request-response pattern
- Higher latency (500ms-1s)
- Unlimited queryable addresses
- Polling-based
- No ordering guarantees

**Data Types**:
- Account state snapshots
- Token supply queries
- Transaction status
- Slot information

**Adapter Responsibility** (`RpcAdapter`):
```rust
pub trait RpcAdapter {
    async fn get_account(&self, address: &Pubkey) -> Result<AccountData>;
    async fn get_multiple_accounts(&self, addresses: &[Pubkey]) -> Result<Vec<AccountData>>;
    async fn poll_for_changes(&self, addresses: &[Pubkey]) -> Receiver<AccountUpdate>;
}
```

**Configuration**:
```toml
[market_data.rpc]
endpoints = ["https://api.mainnet-beta.solana.com"]
polling_interval_ms = 500
max_concurrent_queries = 50
```

See [SOLANA_RPC.md](./SOLANA_RPC.md) for detailed architecture.

---

### 3. DEX Protocol APIs (Supplementary)

**Purpose**: Direct orderbook and liquidity queries

**Characteristics**:
- Protocol-specific formats
- Variable latency
- Orderbook snapshots
- Liquidity information
- Quote generation

**Data Types**:
- Orderbook snapshots
- Available liquidity
- Maker fees, taker fees
- Pool composition

**Adapter Responsibility** (`DexAdapter`):
```rust
pub trait DexAdapter {
    async fn get_orderbook(&self, market: &Pubkey) -> Result<OrderBook>;
    async fn get_quote(&self, swap: &SwapRequest) -> Result<Quote>;
    async fn subscribe_to_updates(&self, market: &Pubkey) -> Receiver<OrderbookUpdate>;
}
```

**Supported DEXes**:
- Jupiter (aggregator)
- Raydium (AMM)
- Orca (AMM)
- Meteora (AMM)
- Phoenix (CLOB)
- OpenBook (CLOB)

See [DEX_INTEGRATIONS.md](./DEX_INTEGRATIONS.md) for detailed protocol integration.

---

## Market Event Types

All market data is normalized into these event types:

```rust
pub enum MarketEvent {
    PriceUpdate(PriceUpdateEvent),
    OrderbookUpdate(OrderbookUpdateEvent),
    LiquidityUpdate(LiquidityUpdateEvent),
    TokenSupplyUpdate(TokenSupplyUpdateEvent),
    PoolStateUpdate(PoolStateUpdateEvent),
}

pub struct PriceUpdateEvent {
    pub timestamp: DateTime<Utc>,
    pub source: DataSource,           // yellowstone, rpc, dex_api
    pub token_pair: TokenPair,
    pub price: Price,
    pub confidence: f64,              // 0.0 to 1.0
    pub volume_24h: Option<f64>,
    pub source_id: String,            // Unique ID for deduplication
}

pub struct OrderbookUpdateEvent {
    pub timestamp: DateTime<Utc>,
    pub market: Pubkey,
    pub bids: Vec<(Price, Quantity)>,
    pub asks: Vec<(Price, Quantity)>,
    pub seqnum: u64,                  // Sequence number for ordering
}

pub struct LiquidityUpdateEvent {
    pub timestamp: DateTime<Utc>,
    pub pool: Pubkey,
    pub token_a: Pubkey,
    pub token_b: Pubkey,
    pub reserve_a: u64,
    pub reserve_b: u64,
}
```

---

## Ingestion Pipeline

```
Data Sources
    ↓
┌─────────────────────────────────┐
│   Raw Event Adapters            │
│ (Yellowstone, RPC, DEX APIs)    │
└──────────┬──────────────────────┘
           ↓
┌─────────────────────────────────┐
│   Format Normalization          │
│ (Convert to common event type)  │
└──────────┬──────────────────────┘
           ↓
┌─────────────────────────────────┐
│   Deduplication                 │
│ (Remove stale/duplicate events) │
└──────────┬──────────────────────┘
           ↓
┌─────────────────────────────────┐
│   Validation                    │
│ (Sanity checks, type safety)    │
└──────────┬──────────────────────┘
           ↓
┌─────────────────────────────────┐
│   Caching                       │
│ (Redis, in-memory)              │
└──────────┬──────────────────────┘
           ↓
┌─────────────────────────────────┐
│   Event Bus                     │
│ (Tokio mpsc broadcast)          │
└──────────┬──────────────────────┘
           ↓
Subscribers
  ├─ Strategy Engine
  ├─ API Consumers
  └─ Persistence Layer
```

---

## Component Responsibilities

### MarketDataManager

Central coordinator for all market data:

```rust
pub struct MarketDataManager {
    yellowstone: YellowstoneAdapter,
    rpc: RpcAdapter,
    dex_clients: DexClients,
    normalizer: EventNormalizer,
    deduplicator: EventDeduplicator,
    cache: MarketDataCache,
    event_bus: Broadcaster<MarketEvent>,
}

impl MarketDataManager {
    pub async fn new(config: &MarketDataConfig) -> Result<Self>;
    
    // Subscribe to all market events
    pub async fn subscribe(&self) -> Receiver<MarketEvent>;
    
    // Subscribe to specific token pair
    pub async fn subscribe_to_token(&self, token: &Pubkey) -> Receiver<PriceUpdate>;
    
    // Query cached orderbook
    pub async fn get_orderbook(&self, market: &Pubkey) -> Result<OrderBook>;
    
    // Query cached price
    pub async fn get_price(&self, token: &Pubkey) -> Result<Price>;
    
    // Manual subscription management
    pub async fn subscribe_accounts(&self, accounts: &[Pubkey]) -> Result<()>;
}
```

### EventNormalizer

Converts source-specific formats to common events:

```rust
pub struct EventNormalizer;

impl EventNormalizer {
    // Normalize Yellowstone account update to market event
    pub fn normalize_yellowstone(&self, update: RawAccountUpdate) -> Result<MarketEvent>;
    
    // Normalize RPC account state to market event
    pub fn normalize_rpc(&self, account: AccountData) -> Result<MarketEvent>;
    
    // Normalize DEX quote to market event
    pub fn normalize_dex_quote(&self, quote: DexQuote) -> Result<MarketEvent>;
}
```

### EventDeduplicator

Removes duplicate and stale events:

```rust
pub struct EventDeduplicator {
    seen_ids: HashMap<String, DateTime<Utc>>,
    ttl: Duration,
}

impl EventDeduplicator {
    pub fn filter(&mut self, event: &MarketEvent) -> bool {
        // Return true if event is new, false if duplicate/stale
    }
    
    pub fn cleanup_stale(&mut self);  // Remove old entries
}
```

### MarketDataCache

In-memory cache for fast access:

```rust
pub struct MarketDataCache {
    prices: Arc<RwLock<HashMap<Pubkey, Price>>>,
    orderbooks: Arc<RwLock<HashMap<Pubkey, OrderBook>>>,
    liquidity: Arc<RwLock<HashMap<Pubkey, LiquidityState>>>,
}

impl MarketDataCache {
    pub async fn get_price(&self, token: &Pubkey) -> Option<Price>;
    pub async fn get_orderbook(&self, market: &Pubkey) -> Option<OrderBook>;
    pub async fn update_price(&self, token: &Pubkey, price: Price);
    pub async fn update_orderbook(&self, market: &Pubkey, book: OrderBook);
}
```

---

## Backpressure & Flow Control

### Queue Management

Each subscription maintains bounded queues:

```rust
pub struct SubscriptionQueue {
    capacity: usize,
    current_size: Arc<AtomicUsize>,
    drop_oldest: bool,    // Discard oldest on overflow
}
```

**Behavior**:
- Normal case: Queue events as they arrive
- Backpressure: If queue full:
  - Option 1: Block sender (apply backpressure)
  - Option 2: Drop oldest events (lose data)
  - Option 3: Skip event (gap in sequence)

**Configuration**:
```toml
[market_data.backpressure]
queue_capacity = 10000      # Max events per subscriber
overflow_strategy = "drop_oldest"  # drop_oldest, block, skip
max_queue_latency_ms = 5000  # Alert if latency exceeds this
```

### Metrics

Monitor ingestion health:

```rust
pub struct IngestionMetrics {
    pub events_received: Counter,
    pub events_processed: Counter,
    pub events_dropped: Counter,
    pub queue_depth: Gauge,
    pub processing_latency: Histogram,
    pub normalizer_errors: Counter,
}
```

---

## High-Availability Strategy

### Source Redundancy

Multiple sources for each data type:

```
PrimaryYellowstone1 ──┐
PrimaryYellowstone2 ──┼──→ Aggregate & Select
FailoverRPC1 ──────────┤
FailoverRPC2 ──────────┘
```

**Selection Logic**:
1. Use Yellowstone if available and healthy
2. Fall back to RPC if Yellowstone unavailable
3. Supplement with DEX APIs for specific pairs
4. Weighted average if multiple sources available

### Health Monitoring

Each source has health status:

```rust
pub enum SourceHealth {
    Healthy,
    Degraded(String),      // Specific degradation reason
    Unavailable,
}

pub struct SourceStatus {
    pub health: SourceHealth,
    pub last_event: DateTime<Utc>,
    pub consecutive_errors: u32,
    pub latency_ms: f64,
}
```

### Automatic Fallover

```rust
let selected_source = match current_source.health() {
    SourceHealth::Healthy => current_source,
    SourceHealth::Degraded(_) if has_alternative => alternative_source,
    _ => primary_fallback,
};
```

---

## Data Quality

### Validation Rules

All events validated before cache update:

```rust
pub struct EventValidator;

impl EventValidator {
    pub fn validate_price_update(event: &PriceUpdateEvent) -> Result<()> {
        // Price sanity checks
        if event.price.value <= 0.0 {
            return Err("Negative price");
        }
        if event.confidence < 0.0 || event.confidence > 1.0 {
            return Err("Invalid confidence");
        }
        Ok(())
    }
    
    pub fn validate_orderbook(book: &OrderBook) -> Result<()> {
        // Orderbook sanity checks
        if book.bids.is_empty() || book.asks.is_empty() {
            return Err("Empty orderbook");
        }
        let highest_bid = book.bids[0].0;
        let lowest_ask = book.asks[0].0;
        if highest_bid >= lowest_ask {
            return Err("Invalid bid-ask spread");
        }
        Ok(())
    }
}
```

### Outlier Detection

Detect suspicious price movements:

```rust
pub struct OutlierDetector {
    historical_prices: VecDeque<Price>,
    window_size: usize,
}

impl OutlierDetector {
    pub fn is_outlier(&self, price: Price) -> bool {
        let mean = self.historical_prices.iter().map(|p| p.value).sum::<f64>()
            / self.historical_prices.len() as f64;
        let stdev = (self.historical_prices.iter()
            .map(|p| (p.value - mean).powi(2))
            .sum::<f64>() / self.historical_prices.len() as f64)
            .sqrt();
        
        // Reject if price > 5 standard deviations from mean
        (price.value - mean).abs() > 5.0 * stdev
    }
}
```

---

## Persistence

### What Gets Persisted

Market data is persisted to PostgreSQL + TimescaleDB:

```sql
-- Market price snapshots (TimescaleDB hypertable)
CREATE TABLE market_prices (
    time TIMESTAMPTZ NOT NULL,
    token_pair TEXT NOT NULL,
    price NUMERIC NOT NULL,
    source TEXT NOT NULL,
    confidence NUMERIC,
    volume_24h NUMERIC
) PARTITION BY RANGE (time);

-- Orderbook snapshots
CREATE TABLE orderbook_snapshots (
    time TIMESTAMPTZ NOT NULL,
    market BYTEA NOT NULL,
    bid_levels INT NOT NULL,
    ask_levels INT NOT NULL,
    data JSONB NOT NULL
) PARTITION BY RANGE (time);
```

### Retention Policy

```toml
[storage.retention]
market_prices_hot = 7        # Days in hot storage (fast access)
market_prices_cold = 365     # Days in cold storage (compressed)
orderbook_snapshots = 90     # Keep orderbook snapshots
granularity_after_days = 7   # Downsample after 7 days
```

---

## Performance Characteristics

### Throughput

| Component | Throughput | Notes |
|-----------|-----------|-------|
| Yellowstone Input | 10,000 events/sec | Validator capacity |
| Normalizer | 50,000 events/sec | CPU-bound |
| Deduplicator | 100,000 events/sec | Memory-bound |
| Cache Update | 100,000 ops/sec | Redis speed |
| Event Broadcast | 50,000 events/sec | Tokio channels |

### Latency (end-to-end)

| Stage | Latency | Notes |
|-------|---------|-------|
| Raw event → Normalization | 0.1ms | Fast transformation |
| Normalization → Deduplication | 0.05ms | Lookup in hashmap |
| Deduplication → Cache | 0.2ms | Redis round-trip |
| Cache → Event broadcast | 0.5ms | Channel send |
| **Total** | **~1ms** | Source to subscribers |

---

## Failure Modes & Recovery

### Failure Mode: Yellowstone Unavailable

- **Detection**: No events for 5 seconds
- **Action**: Switch to RPC polling
- **Recovery**: Resume Yellowstone when available
- **Impact**: Latency increases, throughput may decrease

### Failure Mode: RPC Endpoints Down

- **Detection**: All RPC queries failing
- **Action**: Queue updates, retry with exponential backoff
- **Recovery**: Requests resume when RPC available
- **Impact**: May miss updates if all sources unavailable

### Failure Mode: Cache Full

- **Detection**: Memory usage > 90%
- **Action**: Evict oldest entries
- **Recovery**: Automatic LRU eviction
- **Impact**: May lose some cached data

### Failure Mode: Deduplicator Memory Leak

- **Detection**: Memory growing unbounded
- **Action**: Periodic cleanup of old entries
- **Recovery**: Manual cleanup on detection
- **Impact**: None if cleanup runs regularly

---

## Testing Strategy

See [TESTING_STRATEGY.md](./TESTING_STRATEGY.md) for comprehensive testing.

**Key Test Areas**:
1. **Unit Tests**: Individual adapter functionality
2. **Integration Tests**: Multi-source coordination
3. **Simulation Tests**: Replay historical data
4. **Chaos Tests**: Source failures and recovery
5. **Performance Tests**: Throughput and latency under load

---

## Future Extensions

### Price Feed Integration

Could add external price feeds:
- Pyth Network
- Switchboard
- Chainlink (if available on Solana)

### Machine Learning

Could add ML-based:
- Outlier detection
- Anomaly detection
- Data quality scoring

### Liquidity Aggregation

Could enhance with:
- Cross-DEX liquidity aggregation
- Liquidity pooling analysis
- Optimal routing pre-calculation

---

## Related Documents

- [YELLOWSTONE.md](./YELLOWSTONE.md) - Yellowstone gRPC architecture
- [SOLANA_RPC.md](./SOLANA_RPC.md) - RPC abstraction layer
- [DEX_INTEGRATIONS.md](./DEX_INTEGRATIONS.md) - DEX protocol integration
- [ARCHITECTURE.md](./ARCHITECTURE.md) - System architecture
- [WORKSPACE.md](./WORKSPACE.md) - solstice-market-data crate

---

**Next**: [YELLOWSTONE.md](./YELLOWSTONE.md)
