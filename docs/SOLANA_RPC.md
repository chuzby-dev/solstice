# Solstice Solana RPC Abstraction

**Purpose**: Define RPC client abstraction, connection pooling, and fallback strategies.

**Scope**: RPC endpoint management, query interfaces, transaction submission, and error handling.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Overview

The Solana RPC abstraction layer provides a unified interface to Solana RPC endpoints with automatic failover, connection pooling, rate limiting, and retry logic.

**Responsibilities**:
- Connect to multiple RPC endpoints
- Pool connections for efficiency
- Load balance across endpoints
- Automatic failover on errors
- Retry with exponential backoff
- Rate limiting and quota management
- Query response caching

---

## RPC Client Architecture

```rust
pub struct SolanaRpcClient {
    endpoints: Vec<RpcEndpoint>,
    connection_pool: Arc<ConnectionPool>,
    rate_limiter: Arc<RateLimiter>,
    cache: Arc<ResponseCache>,
}

impl SolanaRpcClient {
    pub async fn new(config: &RpcConfig) -> Result<Self>;
    
    // Account queries
    pub async fn get_account(&self, address: &Pubkey) -> Result<Account>;
    pub async fn get_multiple_accounts(&self, addresses: &[Pubkey]) -> Result<Vec<Account>>;
    
    // Token queries
    pub async fn get_token_supply(&self, mint: &Pubkey) -> Result<u64>;
    pub async fn get_token_accounts_by_owner(&self, owner: &Pubkey) -> Result<Vec<Account>>;
    
    // Transaction queries
    pub async fn get_transaction(&self, sig: &Signature) -> Result<TransactionStatus>;
    pub async fn get_transaction_status(&self, sig: &Signature) -> Result<TransactionStatus>;
    pub async fn get_signatures_for_address(&self, address: &Pubkey) -> Result<Vec<Signature>>;
    
    // State queries
    pub async fn get_slot(&self) -> Result<u64>;
    pub async fn get_block_height(&self) -> Result<u64>;
    pub async fn get_cluster_nodes(&self) -> Result<Vec<ClusterNode>>;
    
    // Transaction submission
    pub async fn send_transaction(&self, tx: &Transaction) -> Result<Signature>;
    pub async fn simulate_transaction(&self, tx: &Transaction) -> Result<SimulationResult>;
}
```

---

## Connection Pooling

### Pool Management

```rust
pub struct ConnectionPool {
    endpoints: Vec<RpcEndpoint>,
    connections: Arc<RwLock<HashMap<String, Vec<RpcConnection>>>>,
    pool_size: usize,
    health_check_interval: Duration,
}

pub struct RpcEndpoint {
    pub url: String,
    pub name: String,
    pub priority: u32,
    pub max_connections: usize,
    pub timeout: Duration,
}

impl ConnectionPool {
    pub async fn get_connection(&self, endpoint: &str) -> Result<RpcConnection>;
    pub async fn return_connection(&self, endpoint: &str, conn: RpcConnection);
    pub async fn health_check(&self) -> Vec<EndpointHealth>;
}
```

### Health Monitoring

```rust
pub struct EndpointHealth {
    pub endpoint: String,
    pub is_healthy: bool,
    pub latency_ms: f64,
    pub error_rate: f64,
    pub last_error: Option<String>,
}

pub struct HealthMonitor {
    check_interval: Duration,
}

impl HealthMonitor {
    pub async fn monitor(&self, pool: &ConnectionPool) {
        loop {
            let health = pool.health_check().await;
            
            for h in health {
                if h.error_rate > 0.1 {
                    warn!("RPC endpoint {} has high error rate: {}", 
                          h.endpoint, h.error_rate);
                }
                if h.latency_ms > 5000.0 {
                    warn!("RPC endpoint {} has high latency: {}ms", 
                          h.endpoint, h.latency_ms);
                }
            }
            
            tokio::time::sleep(self.check_interval).await;
        }
    }
}
```

---

## Load Balancing & Failover

### Endpoint Selection

```rust
pub enum LoadBalancingStrategy {
    RoundRobin,           // Cycle through endpoints
    LeastConnections,     // Use endpoint with fewest active connections
    HealthWeighted,       // Weight by health score
    LatencyWeighted,      // Weight by latency
}

pub struct EndpointSelector {
    strategy: LoadBalancingStrategy,
    endpoints: Vec<RpcEndpoint>,
    health_data: Arc<RwLock<HashMap<String, EndpointHealth>>>,
}

impl EndpointSelector {
    pub async fn select(&self) -> Result<&RpcEndpoint> {
        match self.strategy {
            LoadBalancingStrategy::RoundRobin => {
                self.select_round_robin()
            }
            LoadBalancingStrategy::HealthWeighted => {
                self.select_health_weighted().await
            }
            // ... other strategies
        }
    }
}
```

### Automatic Failover

```rust
pub struct FailoverPolicy {
    max_retries: u32,
    retry_backoff: ExponentialBackoff,
    fallback_strategy: FallbackStrategy,
}

pub enum FallbackStrategy {
    NextEndpoint,         // Try next endpoint
    PrimaryOnly,          // Only retry primary
    RoundRobin,           // Cycle through all
}

impl SolanaRpcClient {
    async fn query_with_failover<T, F>(&self, mut f: F) -> Result<T>
    where
        F: FnMut(&RpcEndpoint) -> BoxFuture<'static, Result<T>>,
    {
        let mut retries = 0;
        let mut last_error = None;
        
        while retries < self.config.max_retries {
            let endpoint = self.selector.select().await?;
            
            match f(endpoint).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);
                    retries += 1;
                    
                    let backoff = self.config.retry_backoff.next_backoff();
                    tokio::time::sleep(backoff).await;
                }
            }
        }
        
        Err(last_error.unwrap())
    }
}
```

---

## Rate Limiting

### Token Bucket Rate Limiter

```rust
pub struct RateLimiter {
    requests_per_second: f64,
    burst_size: u32,
    tokens: Arc<AtomicU32>,
    refill_interval: Duration,
}

impl RateLimiter {
    pub async fn acquire(&self) -> Result<()> {
        loop {
            let current = self.tokens.load(Ordering::Relaxed);
            
            if current > 0 {
                if self.tokens.compare_exchange(
                    current,
                    current - 1,
                    Ordering::Release,
                    Ordering::Relaxed
                ).is_ok() {
                    return Ok(());
                }
            } else {
                // Wait for refill
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    
    async fn refill(&self) {
        loop {
            let new_tokens = ((self.requests_per_second / 100.0) as u32)
                .min(self.burst_size);
            
            self.tokens.store(
                self.tokens.load(Ordering::Relaxed) + new_tokens,
                Ordering::Release
            );
            
            tokio::time::sleep(self.refill_interval).await;
        }
    }
}
```

---

## Query Response Caching

### Cache Strategy

```rust
pub struct ResponseCache {
    entries: Arc<RwLock<HashMap<CacheKey, CacheEntry>>>,
    ttl_config: CacheTtlConfig,
}

pub struct CacheTtlConfig {
    pub get_account: Duration,
    pub get_token_supply: Duration,
    pub get_slot: Duration,
    pub get_block_height: Duration,
}

impl ResponseCache {
    pub async fn get<T>(&self, key: &CacheKey) -> Option<T>
    where
        T: Deserialize,
    {
        let entries = self.entries.read().await;
        
        if let Some(entry) = entries.get(key) {
            if entry.created_at.elapsed() < entry.ttl {
                return Some(entry.value.clone());
            }
        }
        None
    }
    
    pub async fn set(&self, key: CacheKey, value: Vec<u8>, ttl: Duration) {
        let mut entries = self.entries.write().await;
        entries.insert(key, CacheEntry {
            value,
            created_at: Instant::now(),
            ttl,
        });
    }
    
    pub async fn invalidate(&self, key: &CacheKey) {
        let mut entries = self.entries.write().await;
        entries.remove(key);
    }
}
```

---

## Error Handling

### Error Classification

```rust
pub enum RpcError {
    // Transient (retryable)
    Timeout,
    TemporarilyUnavailable,
    RateLimited,
    
    // Permanent (not retryable)
    InvalidRequest,
    AccountNotFound,
    InvalidSignature,
    
    // Unknown (retry with caution)
    InternalServerError(String),
    Unknown(String),
}

impl RpcError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, 
            RpcError::Timeout 
            | RpcError::TemporarilyUnavailable
            | RpcError::RateLimited
        )
    }
}
```

### Retry Logic

```rust
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: f64,
}

impl RetryPolicy {
    pub fn should_retry(&self, error: &RpcError, attempt: u32) -> bool {
        error.is_retryable() && attempt < self.max_retries
    }
    
    pub fn get_delay(&self, attempt: u32) -> Duration {
        let delay_ms = self.initial_delay.as_millis() as f64
            * self.multiplier.powi(attempt as i32);
        
        let delay_ms = delay_ms.min(self.max_delay.as_millis() as f64);
        Duration::from_millis(delay_ms as u64)
    }
}
```

---

## Configuration

```toml
[blockchain.rpc]
# Endpoints
endpoints = [
    "https://api.mainnet-beta.solana.com",
    "https://solana-api.projectserum.com",
    "https://rpc.ankr.com/solana"
]

# Connection pooling
pool_size = 20
connection_timeout_seconds = 10
request_timeout_seconds = 30

# Load balancing
strategy = "health_weighted"    # round_robin, least_connections, health_weighted
health_check_interval_seconds = 30

# Retry policy
max_retries = 3
initial_backoff_ms = 100
max_backoff_ms = 5000

# Rate limiting
requests_per_second = 100
burst_size = 200

# Caching
cache_enabled = true
get_account_cache_ttl_seconds = 5
get_token_supply_cache_ttl_seconds = 10
get_slot_cache_ttl_seconds = 1
```

---

## Query Examples

### Account State Query

```rust
let account = client.get_account(&USDC_MINT).await?;
println!("USDC supply: {}", account.lamports);
```

### Multiple Accounts Query

```rust
let addresses = vec![USDC_MINT, SOL_MINT, ORCA_MINT];
let accounts = client.get_multiple_accounts(&addresses).await?;
```

### Transaction Submission

```rust
let signature = client.send_transaction(&tx).await?;

// Poll for confirmation
loop {
    let status = client.get_transaction_status(&signature).await?;
    match status {
        TransactionStatus::Confirmed => break,
        TransactionStatus::Failed => return Err("Transaction failed"),
        TransactionStatus::Pending => {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}
```

### Simulation

```rust
let sim_result = client.simulate_transaction(&tx).await?;

println!("Compute units used: {}", sim_result.units_consumed);
println!("Return data: {:?}", sim_result.return_data);

if sim_result.err.is_some() {
    println!("Simulation failed: {:?}", sim_result.err);
}
```

---

## Performance Characteristics

| Operation | Latency | Throughput |
|-----------|---------|-----------|
| Get Account | 100-500ms | 100 req/s |
| Get Multiple Accounts | 200-1000ms | 20 req/s |
| Simulate Transaction | 500-2000ms | 10 req/s |
| Send Transaction | 100-300ms | 50 req/s |
| Get Transaction Status | 50-200ms | 200 req/s |

---

## Testing

### Unit Tests

```rust
#[test]
fn test_rate_limiter() {
    let limiter = RateLimiter::new(100.0, 100);
    
    // Should allow 100 requests/sec
    for _ in 0..100 {
        assert!(limiter.try_acquire());
    }
    
    // Should block on 101st request
    assert!(!limiter.try_acquire());
}

#[test]
fn test_retry_logic() {
    let policy = RetryPolicy {
        max_retries: 3,
        initial_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(1000),
        multiplier: 2.0,
    };
    
    assert_eq!(policy.get_delay(0), Duration::from_millis(100));
    assert_eq!(policy.get_delay(1), Duration::from_millis(200));
    assert_eq!(policy.get_delay(2), Duration::from_millis(400));
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_get_account() {
    let client = SolanaRpcClient::from_config(&test_config()).await.unwrap();
    let account = client.get_account(&USDC_MINT).await.unwrap();
    
    assert!(account.lamports > 0);
    assert_eq!(account.owner, spl_token::id());
}

#[tokio::test]
async fn test_failover() {
    // Create client with primary endpoint down
    let client = SolanaRpcClient::from_config(&failover_config()).await.unwrap();
    
    // Should automatically failover to backup
    let account = client.get_account(&USDC_MINT).await.unwrap();
    assert!(account.lamports > 0);
}
```

---

## Related Documents

- [MARKET_DATA.md](./MARKET_DATA.md) - Market data ingestion
- [YELLOWSTONE.md](./YELLOWSTONE.md) - Yellowstone gRPC
- [ARCHITECTURE.md](./ARCHITECTURE.md) - System architecture
- [WORKSPACE.md](./WORKSPACE.md) - solstice-blockchain crate

---

**Next**: [DEX_INTEGRATIONS.md](./DEX_INTEGRATIONS.md)
