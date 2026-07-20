# Solstice DEX Protocol Integrations

**Purpose**: Define integration architecture for DEX protocols and route aggregation.

**Scope**: Jupiter, Raydium, Orca, Meteora, Phoenix, OpenBook integration patterns and unified interface.

**Status**: Draft  
**Version**: 1.0.0-draft

---

## Overview

Solstice integrates with six major Solana DEX protocols to query prices, liquidity, and route trades. A unified abstraction allows strategies to work with any DEX without modification.

**Supported DEXes**:
1. **Jupiter** - DEX aggregator (route finding)
2. **Raydium** - Constant product AMM (USDC, USDT pairs)
3. **Orca** - Concentrated liquidity AMM (stable pairs)
4. **Meteora** - Multi-pool AMM (farming)
5. **Phoenix** - High-performance CLOB (spot trading)
6. **OpenBook** - V3 CLOB (robust orderbook)

---

## Unified DEX Interface

### Trait Definition

```rust
pub trait DexClient: Send + Sync {
    // Get a quote for a swap
    async fn get_quote(&self, request: &QuoteRequest) -> Result<Quote>;
    
    // Get current orderbook/prices
    async fn get_orderbook(&self, market: &Pubkey) -> Result<OrderBook>;
    
    // Get available liquidity
    async fn get_liquidity(&self, market: &Pubkey) -> Result<Liquidity>;
    
    // Build swap instructions
    async fn build_swap_instructions(
        &self,
        swap: &SwapRequest,
        quote: &Quote,
    ) -> Result<Vec<Instruction>>;
    
    // Subscribe to price updates
    async fn subscribe_prices(&self, markets: &[Pubkey]) -> Receiver<PriceUpdate>;
    
    // Get protocol metadata
    fn protocol_name(&self) -> &str;
    fn program_id(&self) -> &Pubkey;
}

pub struct QuoteRequest {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub amount: u64,
    pub slippage_bps: u32,
}

pub struct Quote {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_bps: u32,
    pub price_impact: f64,       // As decimal (0.05 = 5%)
    pub liquidity: u64,
    pub route: Vec<RouteSegment>,
}

pub struct RouteSegment {
    pub dex: String,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub input_amount: u64,
    pub output_amount: u64,
}
```

---

## Protocol Implementations

### 1. Jupiter (Route Aggregator)

**Purpose**: Best route finding across all DEXes

```rust
pub struct JupiterClient {
    http_client: HttpClient,
    api_base: String,
}

impl JupiterClient {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            http_client: HttpClient::new(),
            api_base: "https://api.jup.ag/v6".to_string(),
        })
    }
}

impl DexClient for JupiterClient {
    async fn get_quote(&self, request: &QuoteRequest) -> Result<Quote> {
        let response = self.http_client
            .get(&format!(
                "{}/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}",
                self.api_base,
                request.input_mint,
                request.output_mint,
                request.amount,
                request.slippage_bps
            ))
            .await?;
        
        Ok(Quote::from_jupiter_response(response))
    }
    
    async fn build_swap_instructions(
        &self,
        swap: &SwapRequest,
        quote: &Quote,
    ) -> Result<Vec<Instruction>> {
        // Use Jupiter's swap endpoint to build instructions
        let swap_response = self.http_client
            .post(&format!("{}/swap", self.api_base))
            .json(&SwapRequestBody::from_quote(quote))
            .await?;
        
        Ok(swap_response.tx_instructions)
    }
    
    fn protocol_name(&self) -> &str { "Jupiter" }
    fn program_id(&self) -> &Pubkey { &JUPITER_PROGRAM_ID }
}
```

### 2. Raydium (Constant Product AMM)

**Purpose**: Liquidity for most token pairs

```rust
pub struct RaydiumClient {
    rpc: Arc<SolanaRpcClient>,
    program_id: Pubkey,
}

impl DexClient for RaydiumClient {
    async fn get_quote(&self, request: &QuoteRequest) -> Result<Quote> {
        // Get pool state from blockchain
        let pool_account = self.rpc.get_account(&request.market_pubkey).await?;
        let pool = AmmInfo::unpack(&pool_account.data)?;
        
        // Calculate output using constant product formula
        let output = Self::calculate_output(
            request.amount,
            &pool,
            request.input_mint,
        )?;
        
        Ok(Quote {
            in_amount: request.amount,
            out_amount: output,
            fee_amount: Self::calculate_fee(request.amount),
            fee_bps: 25,  // 0.25%
            price_impact: Self::calculate_impact(&pool, request.amount),
            liquidity: pool.pc_vault_amount,
            route: vec![RouteSegment {
                dex: "Raydium".to_string(),
                input_mint: request.input_mint,
                output_mint: request.output_mint,
                input_amount: request.amount,
                output_amount: output,
            }],
        })
    }
    
    async fn build_swap_instructions(
        &self,
        swap: &SwapRequest,
        quote: &Quote,
    ) -> Result<Vec<Instruction>> {
        // Build Raydium swap instruction
        let instruction = raydium_sdk::instruction::swap(
            &raydium::PROGRAM_ID,
            &swap.pool_pubkey,
            &swap.input_account,
            &swap.output_account,
            swap.payer,
            quote.out_amount,
        )?;
        
        Ok(vec![instruction])
    }
    
    fn protocol_name(&self) -> &str { "Raydium" }
    fn program_id(&self) -> &Pubkey { &RAYDIUM_PROGRAM_ID }
}
```

### 3. Orca (Concentrated Liquidity)

**Purpose**: Stable pairs (USDC/USDT, etc.) with concentrated liquidity

```rust
pub struct OrcaClient {
    rpc: Arc<SolanaRpcClient>,
}

impl DexClient for OrcaClient {
    async fn get_quote(&self, request: &QuoteRequest) -> Result<Quote> {
        // Orca has multiple pool ticks; find relevant range
        let pool_state = self.get_pool_state(&request.market_pubkey).await?;
        
        // Calculate output accounting for concentrated liquidity
        let output = self.calculate_output_with_slippage(
            request.amount,
            &pool_state,
            request.slippage_bps,
        )?;
        
        Ok(Quote {
            in_amount: request.amount,
            out_amount: output,
            fee_amount: Self::calculate_fee(request.amount),
            fee_bps: 4,   // 0.04% for stable pairs
            price_impact: 0.0001,  // Minimal impact on stable pairs
            liquidity: pool_state.total_liquidity,
            route: vec![RouteSegment {
                dex: "Orca".to_string(),
                input_mint: request.input_mint,
                output_mint: request.output_mint,
                input_amount: request.amount,
                output_amount: output,
            }],
        })
    }
    
    fn protocol_name(&self) -> &str { "Orca" }
    fn program_id(&self) -> &Pubkey { &ORCA_PROGRAM_ID }
}
```

### 4. Meteora (Multi-Pool)

**Purpose**: Stable pools with multiple coins and variable fees

```rust
pub struct MeteoraDexClient {
    rpc: Arc<SolanaRpcClient>,
}

// Similar implementation to Orca
// Queries LBPair accounts for current reserves
```

### 5. Phoenix (High-Performance CLOB)

**Purpose**: Native orderbook for tight spreads on active pairs

```rust
pub struct PhoenixClient {
    rpc: Arc<SolanaRpcClient>,
}

impl DexClient for PhoenixClient {
    async fn get_orderbook(&self, market: &Pubkey) -> Result<OrderBook> {
        let market_account = self.rpc.get_account(market).await?;
        let market_state = phoenix::state::load_market_state(&market_account.data)?;
        
        Ok(OrderBook {
            bids: Self::extract_bids(&market_state),
            asks: Self::extract_asks(&market_state),
            timestamp: Utc::now(),
        })
    }
    
    async fn get_quote(&self, request: &QuoteRequest) -> Result<Quote> {
        // Get best available prices from orderbook
        let book = self.get_orderbook(&request.market_pubkey).await?;
        
        let (output, fees) = self.match_order(&book, request.amount)?;
        
        Ok(Quote {
            in_amount: request.amount,
            out_amount: output,
            fee_amount: fees,
            fee_bps: 1,   // Typically very low fees
            price_impact: 0.0001,
            liquidity: book.asks.iter().map(|(_, qty)| qty).sum(),
            route: vec![RouteSegment {
                dex: "Phoenix".to_string(),
                input_mint: request.input_mint,
                output_mint: request.output_mint,
                input_amount: request.amount,
                output_amount: output,
            }],
        })
    }
    
    fn protocol_name(&self) -> &str { "Phoenix" }
    fn program_id(&self) -> &Pubkey { &PHOENIX_PROGRAM_ID }
}
```

### 6. OpenBook (V3 CLOB)

**Purpose**: Robust orderbook with high liquidity

```rust
pub struct OpenBookClient {
    rpc: Arc<SolanaRpcClient>,
}

impl DexClient for OpenBookClient {
    async fn get_orderbook(&self, market: &Pubkey) -> Result<OrderBook> {
        let market_account = self.rpc.get_account(market).await?;
        let market = serum_dex::state::Market::load(
            &market_account,
            &serum_dex::ID,
        )?;
        
        // Load bid/ask slabs
        let (bids, asks) = self.load_bids_asks(&market).await?;
        
        Ok(OrderBook {
            bids: Self::extract_levels(&bids),
            asks: Self::extract_levels(&asks),
            timestamp: Utc::now(),
        })
    }
    
    fn protocol_name(&self) -> &str { "OpenBook" }
    fn program_id(&self) -> &Pubkey { &OPENBOOK_PROGRAM_ID }
}
```

---

## DEX Aggregator

Coordinates quotes from multiple DEXes and finds best route:

```rust
pub struct DexAggregator {
    clients: HashMap<String, Arc<dyn DexClient>>,
    cache: Arc<RouteCache>,
}

impl DexAggregator {
    pub async fn get_best_route(&self, request: &QuoteRequest) -> Result<Quote> {
        // Get quotes from all DEXes
        let mut quotes = vec![];
        
        for (name, client) in &self.clients {
            match client.get_quote(request).await {
                Ok(quote) => quotes.push(quote),
                Err(e) => {
                    warn!("Failed to get quote from {}: {}", name, e);
                }
            }
        }
        
        if quotes.is_empty() {
            return Err("No quotes available".into());
        }
        
        // Select best route (highest output)
        let best = quotes.into_iter()
            .max_by_key(|q| q.out_amount)
            .unwrap();
        
        Ok(best)
    }
    
    pub async fn estimate_slippage(&self, request: &QuoteRequest) -> Result<f64> {
        let quote = self.get_best_route(request).await?;
        
        // Slippage = (expected_output - actual_output) / expected_output
        let expected = self.get_midpoint_price(
            request.input_mint,
            request.output_mint,
        ).await? * (request.amount as f64 / 1_000_000.0);
        
        let slippage = (expected - (quote.out_amount as f64)) / expected;
        Ok(slippage.max(0.0))
    }
}
```

---

## Route Caching

```rust
pub struct RouteCache {
    cache: Arc<RwLock<LruCache<RouteCacheKey, Quote>>>,
    ttl: Duration,
}

pub struct RouteCacheKey {
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
}

impl RouteCache {
    pub async fn get(&self, key: &RouteCacheKey) -> Option<Quote> {
        let cache = self.cache.read().await;
        cache.peek(key).cloned()
    }
    
    pub async fn set(&self, key: RouteCacheKey, quote: Quote) {
        let mut cache = self.cache.write().await;
        cache.put(key, quote);
    }
    
    pub async fn invalidate(&self, input_mint: &Pubkey, output_mint: &Pubkey) {
        // Remove all routes involving these mints
        let mut cache = self.cache.write().await;
        cache.clear();  // Simple: clear entire cache
    }
}
```

---

## Swap Execution

Building multi-leg swaps:

```rust
pub struct SwapBuilder {
    aggregator: DexAggregator,
}

impl SwapBuilder {
    pub async fn build_swap(
        &self,
        input_mint: Pubkey,
        output_mint: Pubkey,
        amount: u64,
    ) -> Result<SwapPlan> {
        // Get best route
        let route = self.aggregator.get_best_route(&QuoteRequest {
            input_mint,
            output_mint,
            amount,
            slippage_bps: 50,  // 0.5% slippage tolerance
        }).await?;
        
        // Build instructions for each route segment
        let mut instructions = vec![];
        
        for segment in &route.route {
            let segment_quote = Quote {
                in_amount: segment.input_amount,
                out_amount: segment.output_amount,
                // ... other fields
                route: vec![segment.clone()],
            };
            
            let segment_instructions = self.aggregator
                .get_client(&segment.dex)?
                .build_swap_instructions(&SwapRequest {
                    input_mint: segment.input_mint,
                    output_mint: segment.output_mint,
                    amount: segment.input_amount,
                }, &segment_quote)
                .await?;
            
            instructions.extend(segment_instructions);
        }
        
        Ok(SwapPlan {
            route,
            instructions,
        })
    }
}
```

---

## Configuration

```toml
[dex_integrations]
enabled_dexes = ["jupiter", "raydium", "orca", "meteora", "phoenix", "openbook"]
default_slippage_bps = 50

[dex_integrations.routing]
strategy = "best_price"        # best_price, lowest_impact
cache_enabled = true
cache_ttl_seconds = 5

[dex_integrations.jupiter]
api_url = "https://api.jup.ag/v6"
timeout_seconds = 10

[dex_integrations.raydium]
program_id = "675kPX9MHTjS2zt1qrXrQVxwwp4kakRTayPyaucjzsw"
rpc_timeout_seconds = 10

[dex_integrations.orca]
program_id = "whirLbMiicVdio4KfUbuVrH8q9PL2yVy51yEXwVjuchB"
concentrated_liquidity = true
```

---

## Performance Targets

| Operation | Target | Notes |
|-----------|--------|-------|
| Get Quote | < 500ms | Parallel queries |
| Best Route Search | < 1000ms | All DEXes queried |
| Route Caching | < 5ms | Cache hit |
| Slippage Estimate | < 100ms | Cached prices |

---

## Testing

```rust
#[tokio::test]
async fn test_jupiter_route() {
    let client = JupiterClient::new().await.unwrap();
    let quote = client.get_quote(&QuoteRequest {
        input_mint: USDC_MINT,
        output_mint: SOL_MINT,
        amount: 1_000_000,
        slippage_bps: 50,
    }).await.unwrap();
    
    assert!(quote.out_amount > 0);
    assert!(quote.fee_bps < 100);
}

#[tokio::test]
async fn test_multi_dex_routing() {
    let aggregator = DexAggregator::new().await.unwrap();
    let route = aggregator.get_best_route(&QuoteRequest {
        input_mint: USDC_MINT,
        output_mint: ORCA_MINT,
        amount: 100_000_000,
        slippage_bps: 50,
    }).await.unwrap();
    
    assert!(!route.route.is_empty());
    assert!(route.out_amount > 0);
}
```

---

## Related Documents

- [MARKET_DATA.md](./MARKET_DATA.md) - Market data ingestion
- [EXECUTION.md](./EXECUTION.md) - Execution planning
- [JITO_INTEGRATION.md](./JITO_INTEGRATION.md) - Bundle execution
- [ARCHITECTURE.md](./ARCHITECTURE.md) - System architecture

---

**Complete**: Market data and blockchain integration layer  
**Next Phase**: Strategy and execution engine

Specification progress: 11/45 documents complete (24%)
