//! Market data manager coordinating ingestion, normalization, and caching.

use crate::cache::MarketDataCache;
use crate::error::{MarketDataResult, MarketDataError};
use solstice_core::types::{Price, OrderBook, TokenPair, MarketEvent};
use std::sync::Arc;
use tracing::{debug, info};

/// Market data manager coordinates all market data operations.
pub struct MarketDataManager {
    cache: Arc<MarketDataCache>,
}

impl MarketDataManager {
    /// Create a new market data manager.
    pub fn new(price_ttl_seconds: u64, orderbook_ttl_seconds: u64) -> Self {
        MarketDataManager {
            cache: Arc::new(MarketDataCache::new(price_ttl_seconds, orderbook_ttl_seconds)),
        }
    }

    /// Get or fetch price for a token pair.
    pub fn get_price(&self, pair: &TokenPair) -> MarketDataResult<Option<Price>> {
        debug!("Getting price for {:?}", pair);
        self.cache.get_price(pair)
    }

    /// Update price in cache.
    pub fn update_price(&self, pair: TokenPair, price: Price) -> MarketDataResult<()> {
        debug!("Updating price for {:?}: {}", pair, price.value);
        self.cache.set_price(pair, price)
    }

    /// Get or fetch orderbook for a market.
    pub fn get_orderbook(&self, pair: &TokenPair) -> MarketDataResult<Option<OrderBook>> {
        debug!("Getting orderbook for {:?}", pair);
        self.cache.get_orderbook(pair)
    }

    /// Update orderbook in cache.
    pub fn update_orderbook(&self, pair: TokenPair, book: OrderBook) -> MarketDataResult<()> {
        debug!("Updating orderbook for {:?}", pair);
        self.cache.set_orderbook(pair, book)
    }

    /// Handle a market event (validate and cache).
    pub fn handle_event(&self, event: MarketEvent) -> MarketDataResult<()> {
        match event {
            MarketEvent::PriceUpdate {
                token_pair,
                price,
                source,
                timestamp,
            } => {
                if price <= 0.0 || !price.is_finite() {
                    return Err(MarketDataError::ValidationError(
                        "Invalid price value".to_string(),
                    ));
                }

                let price_obj = Price::new(price, token_pair.clone(), 0.95);
                self.update_price(token_pair, price_obj)?;
                debug!("Handled price update from {}", source);
                Ok(())
            }

            MarketEvent::OrderbookUpdate { orderbook } => {
                if !orderbook.is_valid() {
                    return Err(MarketDataError::ValidationError(
                        "Invalid orderbook data".to_string(),
                    ));
                }

                self.update_orderbook(orderbook.market.clone(), orderbook)?;
                debug!("Handled orderbook update");
                Ok(())
            }

            MarketEvent::LiquidityUpdate {
                pair,
                available_liquidity,
                timestamp,
            } => {
                debug!("Handled liquidity update for {:?}: {}", pair, available_liquidity);
                Ok(())
            }
        }
    }

    /// Clear all cached data.
    pub fn clear_cache(&self) -> MarketDataResult<()> {
        info!("Clearing market data cache");
        self.cache.clear()
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> MarketDataResult<String> {
        let stats = self.cache.stats()?;
        Ok(format!(
            "Prices: {}, Orderbooks: {}, Total: {}",
            stats.price_entries, stats.orderbook_entries, stats.total_entries
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use chrono::Utc;

    #[test]
    fn test_manager_creation() {
        let manager = MarketDataManager::new(60, 30);
        assert!(manager.cache_stats().is_ok());
    }

    #[test]
    fn test_price_update() {
        let manager = MarketDataManager::new(60, 30);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let price = Price::new(100.0, pair.clone(), 0.95);

        manager.update_price(pair.clone(), price).unwrap();

        let cached = manager.get_price(&pair).unwrap();
        assert!(cached.is_some());
    }

    #[test]
    fn test_event_handling() {
        let manager = MarketDataManager::new(60, 30);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

        let event = MarketEvent::PriceUpdate {
            token_pair: pair.clone(),
            price: 100.0,
            source: "test".to_string(),
            timestamp: Utc::now(),
        };

        manager.handle_event(event).unwrap();

        let cached = manager.get_price(&pair).unwrap();
        assert!(cached.is_some());
    }

    #[test]
    fn test_invalid_price_rejection() {
        let manager = MarketDataManager::new(60, 30);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());

        let event = MarketEvent::PriceUpdate {
            token_pair: pair,
            price: -50.0,  // Invalid: negative
            source: "test".to_string(),
            timestamp: Utc::now(),
        };

        assert!(manager.handle_event(event).is_err());
    }

    #[test]
    fn test_orderbook_update() {
        let manager = MarketDataManager::new(60, 30);
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let book = OrderBook::new(pair.clone(), vec![(100.0, 1000)], vec![(101.0, 1000)]);

        manager.update_orderbook(pair.clone(), book).unwrap();

        let cached = manager.get_orderbook(&pair).unwrap();
        assert!(cached.is_some());
    }
}
