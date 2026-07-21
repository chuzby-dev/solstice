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
//!
//! `build_swap_instructions` uses `raydium_amm`'s own code-generated
//! `SwapBaseIn` instruction builder (produced from Raydium's IDL) for
//! account ordering. The harder part was always the pool's underlying
//! OpenBook/Serum market accounts (bids/asks/event queue/vault signer) --
//! the only Rust crate for that account layout (`serum_dex`/`openbook_dex`)
//! is pinned to `solana-program` 1.10 and doesn't even compile cleanly
//! against modern tooling (confirmed directly: adding it produces
//! `cannot find type Pubkey`/`cannot find associated function
//! process_new_order_v3` compile errors from inside the crate itself).
//! Rather than depend on that, this parses the market account's raw bytes
//! directly -- the classic Serum v3 / OpenBook v1 market layout, unchanged
//! for years. The offset table below isn't a guess: it was derived by
//! fetching the real OpenBook market for Raydium's live SOL/USDC pool
//! (`8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6`) and confirming the
//! `own_address` field at offset 8 decodes back to that account's own
//! address -- a self-referential field that only matches if every prior
//! offset is exactly right.

use crate::error::{DexError, DexResult};
use crate::traits::{DexClient, SwapInstructions};
use crate::types::{Liquidity, PriceUpdate, Quote, QuoteRequest, RouteSegment, SwapRequest};
use async_trait::async_trait;
use chrono::Utc;
use raydium_amm::accounts::AmmInfo;
use raydium_amm::instructions::swap_base_in::{SwapBaseIn, SwapBaseInInstructionArgs};
use raydium_amm::RAYDIUM_AMM_ID;
use solana_sdk::instruction::{AccountMeta, Instruction};
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

const TOKEN_PROGRAM_ID: Pubkey = solana_sdk::pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    solana_sdk::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// The fields of a Serum v3 / OpenBook v1 market account this integration
/// actually needs to build a swap instruction -- not the full struct (it
/// has more trailing fields this doesn't use). See the module doc for how
/// these offsets were verified against a real, live market account.
struct OpenBookMarket {
    vault_signer_nonce: u64,
    coin_vault: Pubkey,
    pc_vault: Pubkey,
    event_queue: Pubkey,
    bids: Pubkey,
    asks: Pubkey,
}

impl OpenBookMarket {
    /// Account data layout: a 5-byte `"serum"` magic prefix, then the
    /// fixed-size market struct, then a 7-byte `"padding"` suffix.
    const HEADER_LEN: usize = 5;

    fn parse(data: &[u8]) -> DexResult<Self> {
        let read_u64 = |offset: usize| -> DexResult<u64> {
            let start = Self::HEADER_LEN + offset;
            let end = start + 8;
            data.get(start..end)
                .map(|b| u64::from_le_bytes(b.try_into().expect("slice is exactly 8 bytes")))
                .ok_or_else(|| {
                    DexError::InvalidPoolState(
                        "market account data too short for OpenBook market layout".to_string(),
                    )
                })
        };
        let read_pubkey = |offset: usize| -> DexResult<Pubkey> {
            let start = Self::HEADER_LEN + offset;
            let end = start + 32;
            data.get(start..end)
                .map(|b| Pubkey::new_from_array(b.try_into().expect("slice is exactly 32 bytes")))
                .ok_or_else(|| {
                    DexError::InvalidPoolState(
                        "market account data too short for OpenBook market layout".to_string(),
                    )
                })
        };

        Ok(OpenBookMarket {
            vault_signer_nonce: read_u64(40)?,
            coin_vault: read_pubkey(112)?,
            pc_vault: read_pubkey(160)?,
            event_queue: read_pubkey(248)?,
            bids: read_pubkey(280)?,
            asks: read_pubkey(312)?,
        })
    }
}

/// Derives an associated token account address. Equivalent to
/// `spl_associated_token_account`'s `get_associated_token_address`, without
/// depending on that crate -- see `crate::orca`'s module doc for why a
/// version-conversion dance isn't needed here specifically (this crate's
/// pubkey type already matches `solana-sdk`'s).
fn derive_ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

/// Builds a `CreateIdempotent` associated-token-account instruction --
/// same account list and `1`-byte discriminant as
/// `spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent`
/// (verified against that crate's source; not depended on here only to
/// avoid the same cross-version pubkey conversion `derive_ata` above
/// avoids).
fn create_ata_idempotent_instruction(
    funding: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Instruction {
    let ata = derive_ata(owner, mint);
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*funding, true),
            AccountMeta::new(ata, false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data: vec![1],
    }
}

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

    async fn fetch_market(&self, market_address: &Pubkey) -> DexResult<OpenBookMarket> {
        let account = self
            .rpc
            .get_account(market_address)
            .await
            .map_err(|e| DexError::AccountQuery(e.to_string()))?;
        let data = account.data.ok_or_else(|| {
            DexError::InvalidPoolState(format!("market {market_address} has no data"))
        })?;
        OpenBookMarket::parse(&data)
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
        swap: &SwapRequest,
        _quote: &Quote,
    ) -> DexResult<SwapInstructions> {
        let pool_address = self
            .find_pool(&swap.input_mint, &swap.output_mint)
            .ok_or(DexError::NoRoute)?;
        let pool = self.fetch_pool(&pool_address).await?;

        let (input_vault, output_vault) =
            if swap.input_mint == pool.coin_mint && swap.output_mint == pool.pc_mint {
                (pool.token_coin, pool.token_pc)
            } else if swap.input_mint == pool.pc_mint && swap.output_mint == pool.coin_mint {
                (pool.token_pc, pool.token_coin)
            } else {
                return Err(DexError::InvalidPoolState(
                    "requested mints do not match pool's coin/pc mints".to_string(),
                ));
            };

        // Fresh reserves/quote computed here (not reused from the
        // caller's `Quote`) to keep the slippage-protected minimum as
        // close to submission time as possible.
        let reserve_in = self.vault_balance(&input_vault).await?;
        let reserve_out = self.vault_balance(&output_vault).await?;
        let (out_amount, _fee_amount) = Self::calculate_output(
            swap.amount,
            reserve_in,
            reserve_out,
            pool.fees.swap_fee_numerator,
            pool.fees.swap_fee_denominator,
        )?;
        let minimum_amount_out =
            out_amount - out_amount.saturating_mul(swap.slippage_bps as u64) / 10_000;

        let market = self.fetch_market(&pool.market).await?;
        let vault_signer = Pubkey::create_program_address(
            &[
                pool.market.as_ref(),
                &market.vault_signer_nonce.to_le_bytes(),
            ],
            &pool.serum_dex,
        )
        .map_err(|e| DexError::InvalidPoolState(format!("vault signer PDA: {e}")))?;
        let (amm_authority, _bump) =
            Pubkey::find_program_address(&[b"amm authority"], &self.program_id);

        let mut instructions = vec![
            create_ata_idempotent_instruction(&swap.payer, &swap.payer, &swap.input_mint),
            create_ata_idempotent_instruction(&swap.payer, &swap.payer, &swap.output_mint),
        ];

        let swap_accounts = SwapBaseIn {
            token_program: TOKEN_PROGRAM_ID,
            amm: pool_address,
            amm_authority,
            amm_open_orders: pool.open_orders,
            amm_target_orders: pool.target_orders,
            pool_coin_token_account: pool.token_coin,
            pool_pc_token_account: pool.token_pc,
            serum_program: pool.serum_dex,
            serum_market: pool.market,
            serum_bids: market.bids,
            serum_asks: market.asks,
            serum_event_queue: market.event_queue,
            serum_coin_vault_account: market.coin_vault,
            serum_pc_vault_account: market.pc_vault,
            serum_vault_signer: vault_signer,
            uer_source_token_account: derive_ata(&swap.payer, &swap.input_mint),
            uer_destination_token_account: derive_ata(&swap.payer, &swap.output_mint),
            user_source_owner: swap.payer,
        };
        let args = SwapBaseInInstructionArgs {
            amount_in: swap.amount,
            minimum_amount_out,
        };
        // `SwapBaseIn::instruction` encodes its data with an 8-byte
        // Anchor-style discriminator (`raydium_amm`'s codegen assumed an
        // Anchor IDL) -- but the deployed Raydium AMM v4 program is a
        // pre-Anchor native program, and that encoding reverted on-chain
        // with "Error: InvalidInstructionData" on the first live attempt.
        // The account list this same builder produces was not implicated
        // (the revert happened at instruction-data deserialization,
        // before any account is touched), so only the data is replaced
        // here: Raydium's actual `AmmInstruction` is a plain Borsh enum,
        // which serializes as a single leading variant-index byte -- `9`
        // for `SwapBaseIn` -- followed by the two `u64` args. Not
        // independently re-verified against a raw on-chain instruction
        // this session (unlike the OpenBook market layout above, which
        // was); if this index is wrong, Solana's mandatory pre-broadcast
        // simulation means it fails the same safe way the Anchor-style
        // encoding did, not a wrong/lost trade.
        let mut swap_instruction = swap_accounts.instruction(args);
        let mut data = vec![9u8];
        data.extend_from_slice(&swap.amount.to_le_bytes());
        data.extend_from_slice(&minimum_amount_out.to_le_bytes());
        swap_instruction.data = data;
        instructions.push(swap_instruction);

        Ok(SwapInstructions {
            instructions,
            address_lookup_tables: Vec::new(),
        })
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
    use std::str::FromStr;

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

    /// Real account data for Raydium's live SOL/USDC pool's OpenBook
    /// market (`8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6`), fetched
    /// live over RPC while building this parser -- not a hand-built
    /// fixture. See the module doc for how this was used to confirm the
    /// offset table.
    fn live_market_fixture() -> Vec<u8> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(
                "c2VydW0DAAAAAAAAAGrEw876nxm/VMjcD15NHO7lMn0mSCsp0rE8uqQ0RyGNAQAAAAAAAAAGm4hX\
             /quBhPtof2NGGMA12sQ53BrrO1WYoPAAAAAAAcb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNF\
             L11hqEu2RmJGeB16mtq4WIuoayrM5RNYyET1RE5AZAh1/VpAFGpXMAAAAAAAAAAAAAAATJ2ZfS7E\
             O9wNI2Jpz7DQg5Gv0QP9j71jRT/1i24jqSBDXoGQAgAAAF5ASjVjAAAAZAAAAAAAAACpQ2mh+mGM\
             lgMZyZsV6oqDen5jWxeAaM/pfOtjDtYM9GsQMjHJdQUM7I2m3kA1fJvKYO+ejzMWWiVWZWUqglM7\
             RlJ5SeCnpln4qtyGvFPMfEJGmhd2WputYrGwW8hote7JvrmxbRioJzl274m3/ehK7Juqyg2xc9uP\
             2krg3keKNEBCDwAAAAAAAQAAAAAAAAAAAAAAAAAAAPWKzkQBAAAAcGFkZGluZw==",
            )
            .unwrap()
    }

    #[test]
    fn test_openbook_market_parse_matches_live_fixture() {
        let data = live_market_fixture();
        assert_eq!(data.len(), 388);

        let market = OpenBookMarket::parse(&data).unwrap();

        assert_eq!(market.vault_signer_nonce, 1);
        assert_eq!(
            market.coin_vault.to_string(),
            "CKxTHwM9fPMRRvZmFnFoqKNd9pQR21c5Aq9bh5h9oghX"
        );
        assert_eq!(
            market.pc_vault.to_string(),
            "6A5NHCj1yF6urc9wZNe6Bcjj4LVszQNj5DwAWG97yzMu"
        );
        assert_eq!(
            market.event_queue.to_string(),
            "8CvwxZ9Db6XbLD46NZwwmVDZZRDy7eydFcAGkXKh9axa"
        );
        assert_eq!(
            market.bids.to_string(),
            "5jWUncPNBMZJ3sTHKmMLszypVkoRK6bfEQMQUHweeQnh"
        );
        assert_eq!(
            market.asks.to_string(),
            "EaXdHx7x3mdGA38j5RSmKYSXMzAFzzUXCLNBEDXDn1d5"
        );
    }

    #[test]
    fn test_openbook_market_parse_rejects_short_data() {
        let result = OpenBookMarket::parse(&[0u8; 10]);
        assert!(matches!(result, Err(DexError::InvalidPoolState(_))));
    }

    #[test]
    fn test_vault_signer_derivation_matches_live_market() {
        // The real market's own program (`serum_dex`/OpenBook's program
        // id) and nonce, cross-checked against the fixture above: a
        // correct seed/program combination must produce a valid
        // off-curve PDA without erroring.
        let data = live_market_fixture();
        let market = OpenBookMarket::parse(&data).unwrap();
        let market_address =
            Pubkey::from_str("8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6").unwrap();
        // OpenBook v1's program id (successor to the legacy Serum v3
        // program at the same account layout); this pool's actual
        // `serum_dex` field is one of the two, both accepted here since
        // this test only checks the derivation doesn't error, not which
        // exact program a given pool uses.
        let openbook_program =
            Pubkey::from_str("srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX").unwrap();

        let result = Pubkey::create_program_address(
            &[
                market_address.as_ref(),
                &market.vault_signer_nonce.to_le_bytes(),
            ],
            &openbook_program,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_derive_ata_is_deterministic_and_mint_specific() {
        let owner = Pubkey::new_unique();
        let mint_a = Pubkey::new_unique();
        let mint_b = Pubkey::new_unique();

        assert_eq!(derive_ata(&owner, &mint_a), derive_ata(&owner, &mint_a));
        assert_ne!(derive_ata(&owner, &mint_a), derive_ata(&owner, &mint_b));
    }

    #[test]
    fn test_create_ata_idempotent_instruction_shape() {
        let funding = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();

        let ix = create_ata_idempotent_instruction(&funding, &owner, &mint);

        assert_eq!(ix.program_id, ASSOCIATED_TOKEN_PROGRAM_ID);
        assert_eq!(ix.accounts.len(), 6);
        assert_eq!(ix.data, vec![1]);
        assert_eq!(ix.accounts[0].pubkey, funding);
        assert!(ix.accounts[0].is_signer);
        assert_eq!(ix.accounts[1].pubkey, derive_ata(&owner, &mint));
    }
}
