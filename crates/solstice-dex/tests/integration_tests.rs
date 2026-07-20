//! Integration tests requiring outbound network access to the live Jupiter
//! API. `#[ignore]`d since this environment cannot reach `api.jup.ag`
//! (only the crates.io registry is reachable). Run with:
//!
//! ```sh
//! cargo test -p solstice-dex -- --ignored
//! ```

use solana_sdk::pubkey::Pubkey;
use solstice_dex::{DexAggregator, DexClient, JupiterClient, QuoteRequest};
use std::str::FromStr;
use std::sync::Arc;

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

#[tokio::test]
#[ignore = "requires outbound network access to api.jup.ag"]
async fn test_jupiter_live_quote() {
    let client = JupiterClient::new().unwrap();
    let request = QuoteRequest::new(
        Pubkey::from_str(USDC_MINT).unwrap(),
        Pubkey::from_str(SOL_MINT).unwrap(),
        1_000_000,
        50,
    );

    let quote = client.get_quote(&request).await.unwrap();
    assert!(quote.out_amount > 0);
    assert!(!quote.route.is_empty());
}

#[tokio::test]
#[ignore = "requires outbound network access to api.jup.ag"]
async fn test_aggregator_with_live_jupiter() {
    let mut aggregator = DexAggregator::new();
    aggregator.register(Arc::new(JupiterClient::new().unwrap()));

    let request = QuoteRequest::new(
        Pubkey::from_str(USDC_MINT).unwrap(),
        Pubkey::from_str(SOL_MINT).unwrap(),
        1_000_000,
        50,
    );

    let route = aggregator.get_best_route(&request).await.unwrap();
    assert!(route.out_amount > 0);
}
