//! Solstice Market Data Ingestion
//!
//! This crate handles market data ingestion from multiple sources (Yellowstone gRPC,
//! Solana RPC, DEX APIs), normalizes the data, caches it, and broadcasts events
//! to subscribers.

pub mod cache;
pub mod error;
pub mod manager;

pub use cache::MarketDataCache;
pub use error::{MarketDataResult, MarketDataError};
pub use manager::MarketDataManager;

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::MarketDataCache>;
    }
}
