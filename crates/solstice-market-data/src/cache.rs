//! In-memory market data cache.

use crate::error::{MarketDataError, MarketDataResult};
use chrono::{DateTime, Utc};
use solstice_core::types::{OrderBook, Price, TokenPair};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::debug;

/// Cache entry with timestamp.
#[derive(Debug, Clone)]
struct CacheEntry<T: Clone> {
    value: T,
    timestamp: DateTime<Utc>,
}

impl<T: Clone> CacheEntry<T> {
    fn new(value: T) -> Self {
        CacheEntry {
            value,
            timestamp: Utc::now(),
        }
    }

    fn age_seconds(&self) -> u64 {
        (Utc::now() - self.timestamp).num_seconds().max(0) as u64
    }

    fn is_expired(&self, ttl_seconds: u64) -> bool {
        self.age_seconds() >= ttl_seconds
    }
}

/// In-memory market data cache with TTL support.
pub struct MarketDataCache {
    prices: Arc<RwLock<HashMap<TokenPair, CacheEntry<Price>>>>,
    orderbooks: Arc<RwLock<HashMap<TokenPair, CacheEntry<OrderBook>>>>,
    price_ttl_seconds: u64,
    orderbook_ttl_seconds: u64,
    max_entries: usize,
}

impl MarketDataCache {
    /// Create a new market data cache.
    pub fn new(price_ttl_seconds: u64, orderbook_ttl_seconds: u64) -> Self {
        MarketDataCache {
            prices: Arc::new(RwLock::new(HashMap::new())),
            orderbooks: Arc::new(RwLock::new(HashMap::new())),
            price_ttl_seconds,
            orderbook_ttl_seconds,
            max_entries: 10000,
        }
    }

    /// Get cached price if available and not expired.
    pub fn get_price(&self, pair: &TokenPair) -> MarketDataResult<Option<Price>> {
        let prices = self
            .prices
            .read()
            .map_err(|_| MarketDataError::CacheError("Lock poisoned".to_string()))?;

        if let Some(entry) = prices.get(pair) {
            if entry.is_expired(self.price_ttl_seconds) {
                debug!("Price cache expired for {:?}", pair);
                return Ok(None);
            }
            return Ok(Some(entry.value));
        }

        Ok(None)
    }

    /// Cache a price update.
    pub fn set_price(&self, pair: TokenPair, price: Price) -> MarketDataResult<()> {
        let mut prices = self
            .prices
            .write()
            .map_err(|_| MarketDataError::CacheError("Lock poisoned".to_string()))?;

        // Simple eviction: clear half of entries if at max capacity
        if prices.len() >= self.max_entries {
            let to_remove = self.max_entries / 2;
            let mut entries: Vec<_> = prices.iter().map(|(k, v)| (*k, v.age_seconds())).collect();
            entries.sort_by_key(|(_, age)| *age);

            for (key, _) in entries.into_iter().take(to_remove) {
                prices.remove(&key);
            }
        }

        prices.insert(pair, CacheEntry::new(price));
        debug!("Cached price for {:?}", pair);
        Ok(())
    }

    /// Get cached orderbook if available and not expired.
    pub fn get_orderbook(&self, pair: &TokenPair) -> MarketDataResult<Option<OrderBook>> {
        let orderbooks = self
            .orderbooks
            .read()
            .map_err(|_| MarketDataError::CacheError("Lock poisoned".to_string()))?;

        if let Some(entry) = orderbooks.get(pair) {
            if entry.is_expired(self.orderbook_ttl_seconds) {
                debug!("Orderbook cache expired for {:?}", pair);
                return Ok(None);
            }
            return Ok(Some(entry.value.clone()));
        }

        Ok(None)
    }

    /// Cache an orderbook update.
    pub fn set_orderbook(&self, pair: TokenPair, book: OrderBook) -> MarketDataResult<()> {
        let mut orderbooks = self
            .orderbooks
            .write()
            .map_err(|_| MarketDataError::CacheError("Lock poisoned".to_string()))?;

        // Simple eviction: clear half of entries if at max capacity
        if orderbooks.len() >= self.max_entries {
            let to_remove = self.max_entries / 2;
            let mut entries: Vec<_> = orderbooks
                .iter()
                .map(|(k, v)| (*k, v.age_seconds()))
                .collect();
            entries.sort_by_key(|(_, age)| *age);

            for (key, _) in entries.into_iter().take(to_remove) {
                orderbooks.remove(&key);
            }
        }

        orderbooks.insert(pair, CacheEntry::new(book));
        debug!("Cached orderbook for {:?}", pair);
        Ok(())
    }

    /// Clear all cached data.
    pub fn clear(&self) -> MarketDataResult<()> {
        self.prices
            .write()
            .map_err(|_| MarketDataError::CacheError("Lock poisoned".to_string()))?
            .clear();

        self.orderbooks
            .write()
            .map_err(|_| MarketDataError::CacheError("Lock poisoned".to_string()))?
            .clear();

        debug!("Cleared all cache entries");
        Ok(())
    }

    /// Get cache statistics.
    pub fn stats(&self) -> MarketDataResult<CacheStats> {
        let prices = self
            .prices
            .read()
            .map_err(|_| MarketDataError::CacheError("Lock poisoned".to_string()))?;

        let orderbooks = self
            .orderbooks
            .read()
            .map_err(|_| MarketDataError::CacheError("Lock poisoned".to_string()))?;

        Ok(CacheStats {
            price_entries: prices.len(),
            orderbook_entries: orderbooks.len(),
            total_entries: prices.len() + orderbooks.len(),
            max_entries: self.max_entries,
        })
    }
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub price_entries: usize,
    pub orderbook_entries: usize,
    pub total_entries: usize,
    pub max_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn test_cache_creation() {
        let cache = MarketDataCache::new(60, 30);
        let stats = cache.stats().unwrap();
        assert_eq!(stats.total_entries, 0);
    }

    #[test]
    fn test_price_caching() {
        let cache = MarketDataCache::new(60, 30);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let price = Price::new(100.0, pair, 0.95);

        cache.set_price(pair, price).unwrap();

        let cached = cache.get_price(&pair).unwrap();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().value, 100.0);
    }

    #[test]
    fn test_cache_expiration() {
        let cache = MarketDataCache::new(0, 0); // Immediate expiration
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let price = Price::new(100.0, pair, 0.95);

        cache.set_price(pair, price).unwrap();

        // Should be expired immediately
        let cached = cache.get_price(&pair).unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_orderbook_caching() {
        let cache = MarketDataCache::new(60, 30);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let book = OrderBook::new(pair, vec![(100.0, 1000)], vec![(101.0, 1000)]);

        cache.set_orderbook(pair, book).unwrap();

        let cached = cache.get_orderbook(&pair).unwrap();
        assert!(cached.is_some());
    }

    #[test]
    fn test_cache_clear() {
        let cache = MarketDataCache::new(60, 30);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let price = Price::new(100.0, pair, 0.95);

        cache.set_price(pair, price).unwrap();
        let stats = cache.stats().unwrap();
        assert_eq!(stats.total_entries, 1);

        cache.clear().unwrap();
        let stats = cache.stats().unwrap();
        assert_eq!(stats.total_entries, 0);
    }
}
