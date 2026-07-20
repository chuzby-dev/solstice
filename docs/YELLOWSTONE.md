# Solstice Yellowstone gRPC Integration

**Purpose**: Define Yellowstone gRPC architecture, connection strategy, and account state streaming.

**Scope**: Yellowstone protocol integration, subscription management, account filtering, and fallback handling.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Overview

[Yellowstone](https://github.com/rpcpool/yellowstone-grpc) is a gRPC-based real-time account state change stream provided by Solana validators. It's the primary market data source for Solstice.

**Advantages**:
- **Real-time atomic updates**: Account changes streamed as they occur
- **Ordered delivery**: Updates ordered by block/transaction
- **Low latency**: 50-200ms from state change to delivery
- **High throughput**: Supports 10,000+ events/second
- **No polling overhead**: Event-driven vs request-response
- **Validator native**: Direct from validator without middleware

---

## Architecture

### Connection Model

```
┌──────────────────────────────────────┐
│     Solstice Application             │
└──────────────┬───────────────────────┘
               │
        (gRPC channels)
               │
    ┌──────────┴──────────┐
    ▼                     ▼
Yellowstone Primary   Yellowstone Secondary
(Primary Endpoint)    (Fallback Endpoint)
    │                     │
    └──────────┬──────────┘
               ▼
    Solana Validator Cluster
```

### Endpoint Configuration

```toml
[market_data.yellowstone]
# Primary endpoint
primary_endpoints = [
    "grpcs://api.mainnet-beta.solana.com:10900",
    "grpcs://rpc-validator.com:10900"
]

# Fallback endpoints
fallback_endpoints = [
    "grpcs://secondary-rpc:10900"
]

# Connection settings
max_subscriptions = 100
subscription_buffer = 10000
timeout_seconds = 30
retry_enabled = true
retry_max_attempts = 3
retry_backoff_multiplier = 2.0
```

---

## Subscription Management

### Account Subscription Types

Solstice subscribes to these account types:

| Account Type | Purpose | Examples |
|-------------|---------|----------|
| Token Mints | Mint info, supply | SOL, USDC, ORCA |
| Token Accounts | Holder balances | User wallets, pool accounts |
| Market Accounts | Orderbooks, prices | Serum, OpenBook, Phoenix |
| Pool Accounts | Liquidity, reserves | Raydium, Orca, Meteora |
| Oracle Accounts | Price data | Switchboard, Pyth |
| Program Accounts | Program state | DEX program state |

### Subscription Lifecycle

```rust
pub struct AccountSubscription {
    pub addresses: Vec<Pubkey>,
    pub filters: Vec<SubscriptionFilter>,
    pub handler: Arc<dyn UpdateHandler>,
    pub active: Arc<AtomicBool>,
}

pub enum SubscriptionFilter {
    MemcmpFilter {
        offset: usize,
        bytes: Vec<u8>,
    },
    LamportFilter {
        min_lamports: u64,
    },
}

impl YellowstoneClient {
    pub async fn subscribe(&self, subscription: AccountSubscription) -> Result<SubscriptionId>;
    pub async fn unsubscribe(&self, id: SubscriptionId) -> Result<()>;
    pub async fn resubscribe(&self, id: SubscriptionId) -> Result<()>;
}
```

### Subscription Optimization

Minimize bandwidth and load:

```rust
pub struct SubscriptionOptimizer {
    // Batch subscriptions
    batch_size: usize,
    batch_interval_ms: u64,
    
    // Filter to relevant data
    filters: Vec<SubscriptionFilter>,
    
    // Deduplicate subscriptions
    seen_accounts: HashSet<Pubkey>,
}

impl SubscriptionOptimizer {
    pub fn optimize_subscriptions(&self, requests: &[SubRequest]) -> Vec<Batch> {
        // Combine multiple subscriptions for same accounts
        // Apply filters to reduce data transfer
        // Batch subscriptions for efficiency
    }
}
```

---

## Message Format & Parsing

### Yellowstone Update Format

```protobuf
message SubscribeUpdateAccount {
    Message account = 1;           // Account data
    Slot slot = 2;                 // Block slot
    bool is_startup = 3;           // Startup slot?
}

message Message {
    bytes pubkey = 1;              // Account address
    bool is_signer = 2;
    bool is_writable = 3;
    uint64 lamports = 4;           // SOL balance
    bytes owner = 5;               // Program owner
    bool executable = 6;
    uint64 rent_epoch = 7;
    bytes data = 8;                // Account data
}
```

### Parsing to Market Events

```rust
pub struct YellowstoneParser;

impl YellowstoneParser {
    pub fn parse_account_update(&self, 
        update: SubscribeUpdateAccount
    ) -> Result<Vec<MarketEvent>> {
        let account = &update.account;
        let events = vec![];
        
        // Identify account type by owner program
        match account.owner {
            token_program::ID => {
                events.push(self.parse_token_account(account)?);
            }
            spl_token_2022::ID => {
                events.push(self.parse_token_2022(account)?);
            }
            raydium_program::ID => {
                events.push(self.parse_raydium_pool(account)?);
            }
            // ... other program types
        }
        
        Ok(events)
    }
}
```

---

## Data Flow

### Update Reception

```
Yellowstone Stream
       ↓
┌──────────────────────────┐
│ gRPC Message Receiver    │
│ (Tokio task)             │
└──────────┬───────────────┘
           ↓
┌──────────────────────────┐
│ Parse Update             │
│ (Protobuf decode)        │
└──────────┬───────────────┘
           ↓
┌──────────────────────────┐
│ Identify Account Type    │
│ (Program owner lookup)   │
└──────────┬───────────────┘
           ↓
┌──────────────────────────┐
│ Type-Specific Parsing    │
│ (Token, Market, Pool)    │
└──────────┬───────────────┘
           ↓
┌──────────────────────────┐
│ Emit Market Events       │
│ (PriceUpdate, etc)       │
└──────────┬───────────────┘
           ↓
        Subscribers
```

---

## Handling Account Types

### Token Mint Accounts

```rust
pub struct TokenMintParser;

impl TokenMintParser {
    pub fn parse(&self, data: &[u8], address: &Pubkey) -> Result<PriceUpdateEvent> {
        let mint = StateWithExtensions::<Mint>::unpack(data)?;
        
        Ok(PriceUpdateEvent {
            token_pair: TokenPair {
                mint: *address,
                quote: USDC,
            },
            // Get price from oracle or other source
            price: self.get_price(&mint)?,
            source: DataSource::Yellowstone,
            timestamp: Utc::now(),
        })
    }
}
```

### Raydium Pool Accounts

```rust
pub struct RaydiumParser;

impl RaydiumParser {
    pub fn parse(&self, data: &[u8], address: &Pubkey) -> Result<MarketEvent> {
        let pool = AmmInfo::unpack(data)?;
        
        Ok(MarketEvent::LiquidityUpdate(LiquidityUpdateEvent {
            pool: *address,
            token_a: pool.token_a_mint,
            token_b: pool.token_b_mint,
            reserve_a: pool.open_orders.native_coin_free,
            reserve_b: pool.open_orders.native_pc_free,
            timestamp: Utc::now(),
        }))
    }
}
```

### OpenBook/Serum Orderbook Accounts

```rust
pub struct OpenBookParser;

impl OpenBookParser {
    pub fn parse(&self, data: &[u8], address: &Pubkey) -> Result<MarketEvent> {
        let bids = Slab::<LeafNode>::load_checked(
            &data[BIDS_OFFSET..],
            &dex::id()
        )?;
        let asks = Slab::<LeafNode>::load_checked(
            &data[ASKS_OFFSET..],
            &dex::id()
        )?;
        
        Ok(MarketEvent::OrderbookUpdate(OrderbookUpdateEvent {
            market: *address,
            bids: Self::extract_bids(&bids),
            asks: Self::extract_asks(&asks),
            timestamp: Utc::now(),
        }))
    }
}
```

---

## Connection Management

### Health Monitoring

```rust
pub struct YellowstoneConnectionMonitor {
    last_update: Arc<Mutex<Instant>>,
    consecutive_errors: Arc<AtomicU32>,
    healthy: Arc<AtomicBool>,
}

impl YellowstoneConnectionMonitor {
    pub async fn monitor(&self) {
        loop {
            let elapsed = self.last_update.lock().await.elapsed();
            
            if elapsed > Duration::from_secs(5) {
                // No updates for 5 seconds
                self.healthy.store(false, Ordering::SeqCst);
                warn!("Yellowstone connection unhealthy");
            } else {
                self.healthy.store(true, Ordering::SeqCst);
            }
            
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    
    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::SeqCst)
    }
}
```

### Automatic Reconnection

```rust
pub struct YellowstoneReconnector {
    endpoint_pool: Vec<String>,
    retry_backoff: ExponentialBackoff,
}

impl YellowstoneReconnector {
    pub async fn maintain_connection(&self) {
        let mut current_endpoint_idx = 0;
        let mut backoff = ExponentialBackoff::new(Duration::from_millis(100));
        
        loop {
            let endpoint = &self.endpoint_pool[current_endpoint_idx];
            
            match self.connect(endpoint).await {
                Ok(stream) => {
                    info!("Connected to Yellowstone: {}", endpoint);
                    backoff.reset();
                    
                    // Handle stream until error
                    if let Err(e) = self.handle_stream(stream).await {
                        warn!("Yellowstone stream error: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Connection failed to {}: {}", endpoint, e);
                    
                    // Try next endpoint
                    current_endpoint_idx = (current_endpoint_idx + 1) % self.endpoint_pool.len();
                    
                    // Exponential backoff
                    let delay = backoff.next_backoff().unwrap();
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}
```

---

## Filtering & Optimization

### Account Filtering

```rust
pub struct AccountFilter {
    // Include these accounts
    include: HashSet<Pubkey>,
    
    // Exclude these accounts
    exclude: HashSet<Pubkey>,
    
    // Include accounts with owner program
    owner_programs: Vec<Pubkey>,
    
    // Include accounts with minimum lamports
    min_lamports: Option<u64>,
}

impl AccountFilter {
    pub fn should_subscribe(&self, address: &Pubkey, owner: &Pubkey) -> bool {
        if self.exclude.contains(address) {
            return false;
        }
        if self.include.contains(address) {
            return true;
        }
        self.owner_programs.contains(owner)
    }
}
```

### Subscription Batching

```rust
pub struct SubscriptionBatcher {
    batch_size: usize,
    batch_interval: Duration,
    pending: Arc<Mutex<Vec<Pubkey>>>,
}

impl SubscriptionBatcher {
    pub async fn batch_subscriptions(&self, addresses: Vec<Pubkey>) {
        for chunk in addresses.chunks(self.batch_size) {
            let mut pending = self.pending.lock().await;
            pending.extend_from_slice(chunk);
            
            if pending.len() >= self.batch_size {
                self.submit_batch(pending.clone()).await;
                pending.clear();
            }
        }
    }
}
```

---

## Backpressure Handling

### Queue Management

```rust
pub struct YellowstoneQueue {
    capacity: usize,
    current_size: Arc<AtomicUsize>,
    buffer: Arc<Mutex<VecDeque<SubscribeUpdateAccount>>>,
}

impl YellowstoneQueue {
    pub async fn enqueue(&self, update: SubscribeUpdateAccount) -> Result<()> {
        let size = self.current_size.fetch_add(1, Ordering::SeqCst);
        
        if size > self.capacity {
            // Backpressure: queue full
            // Options:
            // 1. Block until space available
            // 2. Drop oldest
            // 3. Skip this update
            
            warn!("Yellowstone queue full ({})", size);
            // Implementation choice affects throughput vs data loss
            Ok(())
        } else {
            self.buffer.lock().await.push_back(update);
            Ok(())
        }
    }
}
```

---

## Error Handling

### Transient Errors (Retryable)

- Network timeouts
- Temporary connection loss
- Rate limiting
- gRPC unavailable

**Action**: Retry with exponential backoff

### Permanent Errors (Not Retryable)

- Invalid account subscriptions
- Program not found
- Unsupported data format
- Authentication failure

**Action**: Log and skip, notify operators

### Partial Failures

Some subscriptions succeed, others fail:

```rust
pub struct SubscriptionBatchResult {
    pub successful: Vec<Pubkey>,
    pub failed: Vec<(Pubkey, Error)>,
}

impl YellowstoneClient {
    pub async fn subscribe_batch(&self, 
        addresses: &[Pubkey]
    ) -> SubscriptionBatchResult {
        // Subscribe to all, collect results
        // Return both successes and failures
        // Retry failures separately
    }
}
```

---

## Performance Tuning

### Throughput Optimization

```toml
[market_data.yellowstone.tuning]
# Increase buffer for high throughput
subscription_buffer = 50000

# Batch message processing
batch_size = 100
batch_timeout_ms = 10

# Connection tuning
max_frame_size = 4194304         # 4MB
keepalive_interval_seconds = 10
max_http2_streams = 1000
```

### Latency Optimization

```toml
[market_data.yellowstone.tuning]
# Reduce buffer for lower latency
subscription_buffer = 1000

# Process messages immediately
batch_size = 1
batch_timeout_ms = 0

# Aggressive keepalive
keepalive_interval_seconds = 5
```

---

## Testing Strategy

### Unit Tests

```rust
#[test]
fn test_parse_token_mint() {
    // Create mock Mint account data
    // Parse with YellowstoneParser
    // Verify correct PriceUpdateEvent
}

#[test]
fn test_subscription_batching() {
    // Create batcher with batch_size = 10
    // Add 25 addresses
    // Verify 3 batches created
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_yellowstone_connection() {
    // Connect to Yellowstone
    // Subscribe to test accounts
    // Receive updates
    // Verify timestamps and ordering
}

#[tokio::test]
async fn test_automatic_reconnection() {
    // Simulate connection loss
    // Verify automatic reconnection
    // Verify no data loss
}
```

### Chaos Tests

```rust
#[tokio::test]
async fn test_yellowstone_endpoint_failure() {
    // Primary endpoint unavailable
    // Verify fallback to secondary
    // Verify updates continue
}

#[tokio::test]
async fn test_high_throughput() {
    // Simulate high update rate (10k/sec)
    // Verify no drops or ordering issues
    // Measure latency distribution
}
```

---

## Future Enhancements

### RPC Filtering

Could leverage Yellowstone RPC filters for:
- Mempool monitoring
- Transaction analysis
- Slippage prediction

### Compressed Encoding

Could use Yellowstone's compressed data encoding to:
- Reduce network bandwidth
- Decrease latency
- Handle higher throughput

### Sharding

Could shard subscriptions across multiple connections for:
- Higher concurrency
- Better resource isolation
- Fault isolation

---

## Related Documents

- [MARKET_DATA.md](./MARKET_DATA.md) - Market data ingestion
- [SOLANA_RPC.md](./SOLANA_RPC.md) - RPC fallback
- [ARCHITECTURE.md](./ARCHITECTURE.md) - System architecture
- [WORKSPACE.md](./WORKSPACE.md) - solstice-market-data crate

---

**Next**: [SOLANA_RPC.md](./SOLANA_RPC.md)
