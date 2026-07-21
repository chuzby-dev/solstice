//! Integration tests requiring outbound network access to live DEX APIs
//! and/or a real Solana RPC endpoint. `#[ignore]`d by default so a normal
//! `cargo test` run never depends on network reachability. Run with:
//!
//! ```sh
//! cargo test -p solstice-dex -- --ignored
//! ```

use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::SolanaRpcClient;
use solstice_dex::{
    DexAggregator, DexClient, JupiterClient, OrcaClient, QuoteRequest, RaydiumClient, SwapRequest,
};
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

/// Real read-only verification that Orca's `build_swap_instructions`
/// produces a plausible, well-formed transaction against Orca's actual
/// live SOL/USDC Whirlpool -- never signs or submits anything, so there's
/// no funds risk (same rationale as the Jupiter live tests above).
#[tokio::test]
#[ignore = "requires outbound network access to a real Solana RPC endpoint"]
async fn test_orca_build_swap_instructions_live() {
    const SOL_USDC_WHIRLPOOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";

    let rpc = Arc::new(
        SolanaRpcClient::with_endpoints(vec!["https://api.mainnet-beta.solana.com".to_string()])
            .unwrap(),
    );
    let client = OrcaClient::new(rpc);
    let sol = Pubkey::from_str(SOL_MINT).unwrap();
    let usdc = Pubkey::from_str(USDC_MINT).unwrap();
    let pool = Pubkey::from_str(SOL_USDC_WHIRLPOOL).unwrap();
    client.register_pool(sol, usdc, pool);

    let payer = Pubkey::new_unique();
    let request = QuoteRequest::new(sol, usdc, 10_000_000, 50); // 0.01 SOL
    let quote = client.get_quote(&request).await.unwrap();
    assert!(quote.out_amount > 0);

    let swap = SwapRequest {
        input_mint: sol,
        output_mint: usdc,
        amount: 10_000_000,
        payer,
        slippage_bps: 50,
    };
    let built = client.build_swap_instructions(&swap, &quote).await.unwrap();

    // Two ATA-create instructions plus the swap itself.
    assert_eq!(built.instructions.len(), 3);
    assert!(built
        .instructions
        .iter()
        .any(|ix| ix.program_id == *client.program_id()));
}

/// Same verification for Raydium, against its live SOL/USDC AMM v4 pool.
#[tokio::test]
#[ignore = "requires outbound network access to a real Solana RPC endpoint"]
async fn test_raydium_build_swap_instructions_live() {
    const SOL_USDC_POOL: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";

    let rpc = Arc::new(
        SolanaRpcClient::with_endpoints(vec!["https://api.mainnet-beta.solana.com".to_string()])
            .unwrap(),
    );
    let client = RaydiumClient::new(rpc);
    let sol = Pubkey::from_str(SOL_MINT).unwrap();
    let usdc = Pubkey::from_str(USDC_MINT).unwrap();
    let pool = Pubkey::from_str(SOL_USDC_POOL).unwrap();
    client.register_pool(sol, usdc, pool);

    let payer = Pubkey::new_unique();
    let request = QuoteRequest::new(sol, usdc, 10_000_000, 50); // 0.01 SOL
    let quote = client.get_quote(&request).await.unwrap();
    assert!(quote.out_amount > 0);

    let swap = SwapRequest {
        input_mint: sol,
        output_mint: usdc,
        amount: 10_000_000,
        payer,
        slippage_bps: 50,
    };
    let built = client.build_swap_instructions(&swap, &quote).await.unwrap();

    assert_eq!(built.instructions.len(), 3);
    assert!(built
        .instructions
        .iter()
        .any(|ix| ix.program_id == *client.program_id()));
}
