//! Multi-DEX quote aggregation and route caching.

use crate::error::{DexError, DexResult};
use crate::traits::DexClient;
use crate::types::{Quote, QuoteRequest};
use lru::LruCache;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::warn;

/// Key identifying a cached route: which pair, and how much of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RouteCacheKey {
    pub input_mint: solana_sdk::pubkey::Pubkey,
    pub output_mint: solana_sdk::pubkey::Pubkey,
    pub amount: u64,
}

impl From<&QuoteRequest> for RouteCacheKey {
    fn from(request: &QuoteRequest) -> Self {
        RouteCacheKey {
            input_mint: request.input_mint,
            output_mint: request.output_mint,
            amount: request.amount,
        }
    }
}

struct CachedQuote {
    quote: Quote,
    cached_at: Instant,
}

/// Short-lived cache of recent quotes, keyed by (pair, amount).
pub struct RouteCache {
    cache: RwLock<LruCache<RouteCacheKey, CachedQuote>>,
    ttl: Duration,
}

impl RouteCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1).unwrap());
        RouteCache {
            cache: RwLock::new(LruCache::new(capacity)),
            ttl,
        }
    }

    pub async fn get(&self, key: &RouteCacheKey) -> Option<Quote> {
        let mut cache = self.cache.write().await;
        let entry = cache.get(key)?;
        if entry.cached_at.elapsed() > self.ttl {
            cache.pop(key);
            return None;
        }
        Some(entry.quote.clone())
    }

    pub async fn set(&self, key: RouteCacheKey, quote: Quote) {
        let mut cache = self.cache.write().await;
        cache.put(
            key,
            CachedQuote {
                quote,
                cached_at: Instant::now(),
            },
        );
    }

    /// Drop all cached routes. Route-specific invalidation isn't meaningful
    /// here since a single quote can span multiple hops/mints; any update
    /// that should invalidate one route invalidates the whole cache.
    pub async fn invalidate_all(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

/// Coordinates quotes across multiple DEX clients and selects the best route.
pub struct DexAggregator {
    clients: HashMap<String, Arc<dyn DexClient>>,
    cache: RouteCache,
}

impl DexAggregator {
    pub fn new() -> Self {
        DexAggregator {
            clients: HashMap::new(),
            cache: RouteCache::new(1_000, Duration::from_secs(5)),
        }
    }

    pub fn with_cache(cache_capacity: usize, cache_ttl: Duration) -> Self {
        DexAggregator {
            clients: HashMap::new(),
            cache: RouteCache::new(cache_capacity, cache_ttl),
        }
    }

    pub fn register(&mut self, client: Arc<dyn DexClient>) {
        self.clients
            .insert(client.protocol_name().to_string(), client);
    }

    pub fn get_client(&self, name: &str) -> DexResult<Arc<dyn DexClient>> {
        self.clients
            .get(name)
            .cloned()
            .ok_or_else(|| DexError::UnknownDex(name.to_string()))
    }

    pub fn registered_dexes(&self) -> Vec<&str> {
        self.clients.keys().map(String::as_str).collect()
    }

    /// Query every registered DEX for a quote and return the one with the
    /// highest output, using the route cache when a fresh entry exists.
    pub async fn get_best_route(&self, request: &QuoteRequest) -> DexResult<Quote> {
        let cache_key = RouteCacheKey::from(request);
        if let Some(cached) = self.cache.get(&cache_key).await {
            return Ok(cached);
        }

        if self.clients.is_empty() {
            return Err(DexError::NoRoute);
        }

        let mut handles = Vec::with_capacity(self.clients.len());
        for (name, client) in &self.clients {
            let client = Arc::clone(client);
            let name = name.clone();
            let request = request.clone();
            handles.push(tokio::spawn(async move {
                (name, client.get_quote(&request).await)
            }));
        }

        let mut best: Option<Quote> = None;
        for handle in handles {
            let (name, result) = match handle.await {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("DEX quote task panicked: {}", e);
                    continue;
                }
            };
            match result {
                Ok(quote) => {
                    if best
                        .as_ref()
                        .map(|b| quote.out_amount > b.out_amount)
                        .unwrap_or(true)
                    {
                        best = Some(quote);
                    }
                }
                Err(e) => warn!("Failed to get quote from {}: {}", name, e),
            }
        }

        let best = best.ok_or(DexError::NoRoute)?;
        self.cache.set(cache_key, best.clone()).await;
        Ok(best)
    }

    /// Estimate slippage of the best available route against the simple
    /// midpoint price implied by that same route's first leg.
    pub async fn estimate_slippage(&self, request: &QuoteRequest) -> DexResult<f64> {
        let quote = self.get_best_route(request).await?;
        if quote.in_amount == 0 {
            return Ok(0.0);
        }

        let Some(first_leg) = quote.route.first() else {
            return Ok(0.0);
        };
        if first_leg.input_amount == 0 {
            return Ok(0.0);
        }

        let implied_price = first_leg.output_amount as f64 / first_leg.input_amount as f64;
        let expected_out = implied_price * request.amount as f64;
        if expected_out <= 0.0 {
            return Ok(0.0);
        }

        let slippage = (expected_out - quote.out_amount as f64) / expected_out;
        Ok(slippage.max(0.0))
    }
}

impl Default for DexAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DexResult as Result;
    use crate::traits::SwapInstructions;
    use crate::types::{Liquidity, PriceUpdate, RouteSegment, SwapRequest};
    use async_trait::async_trait;
    use solana_sdk::pubkey::Pubkey;
    use tokio::sync::mpsc;

    struct MockDex {
        name: &'static str,
        program_id: Pubkey,
        out_amount: u64,
    }

    #[async_trait]
    impl DexClient for MockDex {
        async fn get_quote(&self, request: &QuoteRequest) -> Result<Quote> {
            Ok(Quote {
                in_amount: request.amount,
                out_amount: self.out_amount,
                fee_amount: 0,
                fee_bps: 0,
                price_impact: 0.0,
                liquidity: self.out_amount,
                route: vec![RouteSegment {
                    dex: self.name.to_string(),
                    input_mint: request.input_mint,
                    output_mint: request.output_mint,
                    input_amount: request.amount,
                    output_amount: self.out_amount,
                }],
                timestamp: chrono::Utc::now(),
            })
        }

        async fn get_orderbook(&self, _market: &Pubkey) -> Result<solstice_core::types::OrderBook> {
            Err(DexError::NoQuote)
        }

        async fn get_liquidity(&self, _market: &Pubkey) -> Result<Liquidity> {
            Err(DexError::NoQuote)
        }

        async fn build_swap_instructions(
            &self,
            _swap: &SwapRequest,
            _quote: &Quote,
        ) -> Result<SwapInstructions> {
            Ok(SwapInstructions::default())
        }

        async fn subscribe_prices(&self, _markets: &[Pubkey]) -> mpsc::Receiver<PriceUpdate> {
            let (_tx, rx) = mpsc::channel(1);
            rx
        }

        fn protocol_name(&self) -> &str {
            self.name
        }

        fn program_id(&self) -> &Pubkey {
            &self.program_id
        }
    }

    fn sample_request() -> QuoteRequest {
        QuoteRequest::new(Pubkey::new_unique(), Pubkey::new_unique(), 1_000_000, 50)
    }

    #[tokio::test]
    async fn test_no_route_with_no_clients() {
        let aggregator = DexAggregator::new();
        let result = aggregator.get_best_route(&sample_request()).await;
        assert!(matches!(result, Err(DexError::NoRoute)));
    }

    #[tokio::test]
    async fn test_best_route_picks_highest_output() {
        let mut aggregator = DexAggregator::new();
        aggregator.register(Arc::new(MockDex {
            name: "Low",
            program_id: Pubkey::new_unique(),
            out_amount: 100,
        }));
        aggregator.register(Arc::new(MockDex {
            name: "High",
            program_id: Pubkey::new_unique(),
            out_amount: 200,
        }));

        let best = aggregator.get_best_route(&sample_request()).await.unwrap();
        assert_eq!(best.out_amount, 200);
        assert_eq!(best.route[0].dex, "High");
    }

    #[tokio::test]
    async fn test_route_cache_hit() {
        let mut aggregator = DexAggregator::new();
        aggregator.register(Arc::new(MockDex {
            name: "Only",
            program_id: Pubkey::new_unique(),
            out_amount: 500,
        }));

        let request = sample_request();
        let first = aggregator.get_best_route(&request).await.unwrap();
        let second = aggregator.get_best_route(&request).await.unwrap();
        assert_eq!(first.out_amount, second.out_amount);
    }

    #[tokio::test]
    async fn test_get_client_unknown() {
        let aggregator = DexAggregator::new();
        assert!(matches!(
            aggregator.get_client("Nonexistent"),
            Err(DexError::UnknownDex(_))
        ));
    }

    #[tokio::test]
    async fn test_estimate_slippage_no_impact() {
        let mut aggregator = DexAggregator::new();
        aggregator.register(Arc::new(MockDex {
            name: "Only",
            program_id: Pubkey::new_unique(),
            out_amount: 1_000_000,
        }));

        let request = QuoteRequest::new(Pubkey::new_unique(), Pubkey::new_unique(), 1_000_000, 50);
        let slippage = aggregator.estimate_slippage(&request).await.unwrap();
        assert!(slippage.abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_route_cache_expires() {
        let cache = RouteCache::new(10, Duration::from_millis(10));
        let key = RouteCacheKey {
            input_mint: Pubkey::new_unique(),
            output_mint: Pubkey::new_unique(),
            amount: 1,
        };
        let quote = Quote {
            in_amount: 1,
            out_amount: 1,
            fee_amount: 0,
            fee_bps: 0,
            price_impact: 0.0,
            liquidity: 1,
            route: vec![],
            timestamp: chrono::Utc::now(),
        };

        cache.set(key.clone(), quote).await;
        assert!(cache.get(&key).await.is_some());

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(cache.get(&key).await.is_none());
    }
}
