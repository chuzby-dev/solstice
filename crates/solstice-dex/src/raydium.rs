//! Raydium AMM v4 (constant product) integration.
//!
//! Pool addresses aren't derivable from a mint pair alone, so callers must
//! register known pools via [`RaydiumClient::register_pool`] before
//! quoting them (typically from a config file or a pool-discovery job, per
//! `docs/DEX_INTEGRATIONS.md`'s `RouteCache`-adjacent design; discovery
//! itself is out of scope here).
//!
//! `get_quote` is a real, fully-wired implementation: it fetches the pool
//! account and both vault token accounts over RPC and applies Raydium's
//! actual constant-product formula with its actual on-chain fee.
//! `build_swap_instructions` is intentionally *not* implemented: Raydium's
//! `SwapBaseIn` instruction also requires the pool's underlying
//! OpenBook/Serum market accounts (bids/asks/event queue/vault signer),
//! and the only crate for that account layout (`serum_dex`) is pinned to
//! a 2022-era Solana SDK incompatible with this workspace's solana-sdk 2.x.
//! Hand-rolling that layout from memory risks producing a wrong account
//! list for a real, funds-moving instruction, so this returns a clear
//! error instead of a guess.

use crate::error::{DexError, DexResult};
use crate::traits::{DexClient, SwapInstructions};
use crate::types::{Liquidity, PriceUpdate, Quote, QuoteRequest, RouteSegment, SwapRequest};
use async_trait::async_trait;
use chrono::Utc;
use raydium_amm::accounts::AmmInfo;
use raydium_amm::RAYDIUM_AMM_ID;
use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::SolanaRpcClient;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

/// SPL Token account `amount` field offset (after 32-byte mint + 32-byte
/// owner). This layout has been stable since the SPL Token Program's
/// original release.
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const TOKEN_ACCOUNT_AMOUNT_LEN: usize = 8;

pub struct RaydiumClient {
    rpc: Arc<SolanaRpcClient>,
    program_id: Pubkey,
    /// (base_mint, quote_mint) -> pool address, in either order.
    pools: RwLock<HashMap<(Pubkey, Pubkey), Pubkey>>,
}

impl RaydiumClient {
    pub fn new(rpc: Arc<SolanaRpcClient>) -> Self {
        RaydiumClient {
            rpc,
            program_id: RAYDIUM_AMM_ID,
            pools: RwLock::new(HashMap::new()),
        }
    }

    /// Register a known pool for a mint pair. Order of `mint_a`/`mint_b`
    /// doesn't matter; lookups check both orderings.
    pub fn register_pool(&self, mint_a: Pubkey, mint_b: Pubkey, pool: Pubkey) {
        if let Ok(mut pools) = self.pools.write() {
            pools.insert((mint_a, mint_b), pool);
            pools.insert((mint_b, mint_a), pool);
        }
    }

    fn find_pool(&self, mint_a: &Pubkey, mint_b: &Pubkey) -> Option<Pubkey> {
        self.pools.read().ok()?.get(&(*mint_a, *mint_b)).copied()
    }

    async fn fetch_pool(&self, pool_address: &Pubkey) -> DexResult<AmmInfo> {
        let account = self
            .rpc
            .get_account(pool_address)
            .await
            .map_err(|e| DexError::AccountQuery(e.to_string()))?;
        let data = account.data.ok_or_else(|| {
            DexError::InvalidPoolState(format!("pool {pool_address} has no data"))
        })?;

        AmmInfo::from_bytes(&data)
            .map_err(|e| DexError::InvalidPoolState(format!("failed to decode pool: {e}")))
    }

    async fn vault_balance(&self, vault: &Pubkey) -> DexResult<u64> {
        let account = self
            .rpc
            .get_account(vault)
            .await
            .map_err(|e| DexError::AccountQuery(e.to_string()))?;
        let data = account
            .data
            .ok_or_else(|| DexError::InvalidPoolState(format!("vault {vault} has no data")))?;

        let end = TOKEN_ACCOUNT_AMOUNT_OFFSET + TOKEN_ACCOUNT_AMOUNT_LEN;
        if data.len() < end {
            return Err(DexError::InvalidPoolState(format!(
                "vault {vault} data too short for SPL token account layout"
            )));
        }

        let bytes: [u8; 8] = data[TOKEN_ACCOUNT_AMOUNT_OFFSET..end]
            .try_into()
            .expect("slice is exactly 8 bytes");
        Ok(u64::from_le_bytes(bytes))
    }

    /// Constant-product output for swapping `amount_in` of the input side
    /// into the output side, given both reserves and Raydium's actual
    /// on-chain swap fee.
    fn calculate_output(
        amount_in: u64,
        reserve_in: u64,
        reserve_out: u64,
        fee_numerator: u64,
        fee_denominator: u64,
    ) -> DexResult<(u64, u64)> {
        if reserve_in == 0 || reserve_out == 0 {
            return Err(DexError::NoQuote);
        }
        if fee_denominator == 0 {
            return Err(DexError::InvalidPoolState(
                "fee denominator is zero".to_string(),
            ));
        }

        let amount_in = amount_in as u128;
        let reserve_in = reserve_in as u128;
        let reserve_out = reserve_out as u128;
        let fee_numerator = fee_numerator as u128;
        let fee_denominator = fee_denominator as u128;

        let fee_amount = amount_in * fee_numerator / fee_denominator;
        let amount_in_after_fee = amount_in - fee_amount;
        let numerator = amount_in_after_fee * reserve_out;
        let denominator = reserve_in + amount_in_after_fee;
        let amount_out = numerator / denominator;

        Ok((amount_out as u64, fee_amount as u64))
    }
}

#[async_trait]
impl DexClient for RaydiumClient {
    async fn get_quote(&self, request: &QuoteRequest) -> DexResult<Quote> {
        let pool_address = self
            .find_pool(&request.input_mint, &request.output_mint)
            .ok_or(DexError::NoRoute)?;

        let pool = self.fetch_pool(&pool_address).await?;

        let (input_is_coin, input_vault, output_vault) =
            if request.input_mint == pool.coin_mint && request.output_mint == pool.pc_mint {
                (true, pool.token_coin, pool.token_pc)
            } else if request.input_mint == pool.pc_mint && request.output_mint == pool.coin_mint {
                (false, pool.token_pc, pool.token_coin)
            } else {
                return Err(DexError::InvalidPoolState(
                    "requested mints do not match pool's coin/pc mints".to_string(),
                ));
            };
        let _ = input_is_coin;

        let reserve_in = self.vault_balance(&input_vault).await?;
        let reserve_out = self.vault_balance(&output_vault).await?;

        let (out_amount, fee_amount) = Self::calculate_output(
            request.amount,
            reserve_in,
            reserve_out,
            pool.fees.swap_fee_numerator,
            pool.fees.swap_fee_denominator,
        )?;

        let fee_bps = if pool.fees.swap_fee_denominator == 0 {
            0
        } else {
            ((pool.fees.swap_fee_numerator as u128 * 10_000)
                / pool.fees.swap_fee_denominator as u128)
                .min(10_000) as u32
        };

        // Price impact: how far the post-trade price has moved from the
        // pre-trade spot price, as a fraction of the spot price.
        let spot_price = reserve_out as f64 / reserve_in as f64;
        let execution_price = out_amount as f64 / request.amount as f64;
        let price_impact = if spot_price > 0.0 {
            ((spot_price - execution_price) / spot_price).max(0.0)
        } else {
            0.0
        };

        Ok(Quote {
            in_amount: request.amount,
            out_amount,
            fee_amount,
            fee_bps,
            price_impact,
            liquidity: reserve_out,
            route: vec![RouteSegment {
                dex: "Raydium".to_string(),
                input_mint: request.input_mint,
                output_mint: request.output_mint,
                input_amount: request.amount,
                output_amount: out_amount,
            }],
            timestamp: Utc::now(),
        })
    }

    async fn get_orderbook(&self, _market: &Pubkey) -> DexResult<solstice_core::types::OrderBook> {
        // Raydium AMM v4 has no discrete orderbook; price is derived from
        // constant-product reserves via get_quote.
        Err(DexError::NoQuote)
    }

    async fn get_liquidity(&self, market: &Pubkey) -> DexResult<Liquidity> {
        let pool = self.fetch_pool(market).await?;
        let coin_reserve = self.vault_balance(&pool.token_coin).await?;
        let pc_reserve = self.vault_balance(&pool.token_pc).await?;

        Ok(Liquidity {
            base_reserve: coin_reserve,
            quote_reserve: pc_reserve,
            timestamp: Utc::now(),
        })
    }

    async fn build_swap_instructions(
        &self,
        _swap: &SwapRequest,
        _quote: &Quote,
    ) -> DexResult<SwapInstructions> {
        Err(DexError::InvalidPoolState(
            "Raydium swap instruction building requires the pool's OpenBook/Serum market \
             accounts (bids/asks/event queue/vault signer), which this integration does not \
             yet resolve — see module docs for why"
                .to_string(),
        ))
    }

    async fn subscribe_prices(&self, _markets: &[Pubkey]) -> mpsc::Receiver<PriceUpdate> {
        // No push feed for on-chain AMM state without the Yellowstone
        // account-filter wiring (Phase 1.2's adapter); price updates for
        // registered pools are consumed via that pipeline, not here.
        let (_tx, rx) = mpsc::channel(1);
        rx
    }

    fn protocol_name(&self) -> &str {
        "Raydium"
    }

    fn program_id(&self) -> &Pubkey {
        &self.program_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_output_constant_product() {
        // 1_000_000 in, 25/10000 fee, reserves 100M/200M -> out < 2_000_000
        // (proportional share) due to price impact + fee.
        let (out, fee) =
            RaydiumClient::calculate_output(1_000_000, 100_000_000, 200_000_000, 25, 10_000)
                .unwrap();

        assert_eq!(fee, 2_500);
        assert!(out > 0 && out < 2_000_000);
    }

    #[test]
    fn test_calculate_output_zero_reserves() {
        let result = RaydiumClient::calculate_output(1_000, 0, 100, 25, 10_000);
        assert!(matches!(result, Err(DexError::NoQuote)));
    }

    #[test]
    fn test_calculate_output_zero_fee_denominator() {
        let result = RaydiumClient::calculate_output(1_000, 100, 100, 0, 0);
        assert!(matches!(result, Err(DexError::InvalidPoolState(_))));
    }

    #[test]
    fn test_register_and_find_pool() {
        let rpc = Arc::new(
            SolanaRpcClient::with_endpoints(vec!["http://localhost:8899".to_string()]).unwrap(),
        );
        let client = RaydiumClient::new(rpc);

        let mint_a = Pubkey::new_unique();
        let mint_b = Pubkey::new_unique();
        let pool = Pubkey::new_unique();

        assert!(client.find_pool(&mint_a, &mint_b).is_none());
        client.register_pool(mint_a, mint_b, pool);

        assert_eq!(client.find_pool(&mint_a, &mint_b), Some(pool));
        assert_eq!(client.find_pool(&mint_b, &mint_a), Some(pool));
    }

    #[test]
    fn test_protocol_metadata() {
        let rpc = Arc::new(
            SolanaRpcClient::with_endpoints(vec!["http://localhost:8899".to_string()]).unwrap(),
        );
        let client = RaydiumClient::new(rpc);

        assert_eq!(client.protocol_name(), "Raydium");
        assert_eq!(*client.program_id(), RAYDIUM_AMM_ID);
    }
}
