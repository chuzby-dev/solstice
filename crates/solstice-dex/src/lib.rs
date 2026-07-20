//! Solstice DEX Integration Layer
//!
//! Unified interface over Solana DEX protocols: quotes, orderbooks,
//! liquidity, and swap instruction building. See
//! `docs/DEX_INTEGRATIONS.md`.

pub mod aggregator;
pub mod error;
pub mod jupiter;
pub mod orca;
pub mod raydium;
pub mod traits;
pub mod types;

pub use aggregator::{DexAggregator, RouteCache, RouteCacheKey};
pub use error::{DexError, DexResult};
pub use jupiter::JupiterClient;
pub use orca::OrcaClient;
pub use raydium::RaydiumClient;
pub use traits::DexClient;
pub use types::{Liquidity, PriceUpdate, Quote, QuoteRequest, RouteSegment, SwapPlan, SwapRequest};

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::DexAggregator>;
    }
}
