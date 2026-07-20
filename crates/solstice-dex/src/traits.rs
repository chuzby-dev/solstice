//! Unified DEX client interface. See `docs/DEX_INTEGRATIONS.md`.

use crate::error::DexResult;
use crate::types::{Liquidity, PriceUpdate, Quote, QuoteRequest, SwapRequest};
use async_trait::async_trait;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use tokio::sync::mpsc;

/// A single DEX/aggregator integration.
///
/// Implementors must be safe to share across tasks (`Send + Sync`) since a
/// [`crate::aggregator::DexAggregator`] holds them behind `Arc<dyn DexClient>`
/// and queries all configured DEXes concurrently.
#[async_trait]
pub trait DexClient: Send + Sync {
    /// Get a quote for a swap.
    async fn get_quote(&self, request: &QuoteRequest) -> DexResult<Quote>;

    /// Get the current orderbook for a market, if this DEX exposes one
    /// (AMMs without a discrete orderbook may return
    /// [`DexError::NoQuote`](crate::error::DexError::NoQuote)).
    async fn get_orderbook(&self, market: &Pubkey) -> DexResult<solstice_core::types::OrderBook>;

    /// Get available liquidity for a market.
    async fn get_liquidity(&self, market: &Pubkey) -> DexResult<Liquidity>;

    /// Build swap instructions from an already-obtained quote.
    async fn build_swap_instructions(
        &self,
        swap: &SwapRequest,
        quote: &Quote,
    ) -> DexResult<Vec<Instruction>>;

    /// Subscribe to price updates for the given markets. The returned
    /// channel closes when the subscription ends (connection loss for
    /// stream-based DEXes, or task shutdown for polling-based ones).
    async fn subscribe_prices(&self, markets: &[Pubkey]) -> mpsc::Receiver<PriceUpdate>;

    /// Human-readable protocol name (e.g. `"Jupiter"`).
    fn protocol_name(&self) -> &str;

    /// The on-chain program this DEX executes swaps through.
    fn program_id(&self) -> &Pubkey;
}
