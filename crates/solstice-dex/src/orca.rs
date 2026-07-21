//! Orca Whirlpools (concentrated liquidity AMM) integration.
//!
//! Pool addresses aren't derivable from a mint pair alone (same reasoning
//! as [`crate::raydium`]), so callers must register known pools via
//! [`OrcaClient::register_pool`] first.
//!
//! `get_quote` and `get_liquidity` are real, fully-wired implementations:
//! they fetch the pool and surrounding tick-array accounts over RPC and
//! delegate the actual concentrated-liquidity math (tick crossing, fee
//! application, sqrt-price arithmetic) to `orca_whirlpools_core` — Orca's
//! own vetted implementation of that math, not a reimplementation of it
//! here. `build_swap_instructions` uses `orca_whirlpools_client`'s own
//! code-generated `SwapV2` instruction builder (`generated::instructions`,
//! produced directly from Orca's IDL) for account ordering, rather than
//! hand-assembling it — the account list (including the three tick-array
//! slots' order) comes from that generated code, not from guessing.
//! Payer token accounts are derived and idempotently created via
//! `spl-associated-token-account-interface`, matching the "CreateIdempotent"
//! step already observed in real Jupiter-routed transactions this session.
//! Both mints are assumed to use the classic SPL Token program (true for
//! SOL/USDC and the vast majority of pools); Token-2022 mints would need
//! each mint's actual owner program looked up instead of this fixed
//! assumption.

use crate::error::{DexError, DexResult};
use crate::traits::{DexClient, SwapInstructions};
use crate::types::{Liquidity, PriceUpdate, Quote, QuoteRequest, RouteSegment, SwapRequest};
use async_trait::async_trait;
use chrono::Utc;
use orca_whirlpools_client::{
    get_oracle_address, get_tick_array_address, SwapV2, SwapV2InstructionArgs, Whirlpool,
};
use orca_whirlpools_core::{
    get_tick_array_start_tick_index, swap_quote_by_input_token, TickArrayFacade, TickArrays,
    TICK_ARRAY_SIZE,
};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::SolanaRpcClient;
use spl_associated_token_account_interface::address::get_associated_token_address;
use spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const TOKEN_ACCOUNT_AMOUNT_LEN: usize = 8;

/// `orca_whirlpools_client` (and `spl-associated-token-account-interface`,
/// used below) pin `solana-pubkey`/`solana-instruction` on the `3.x` line,
/// one major version ahead of this workspace's `solana-sdk` 2.x — Cargo
/// resolves them as distinct types, so every value crossing that boundary
/// needs an explicit conversion. Raydium's equivalent crate (see
/// `crate::raydium`) happens to resolve to the same 2.x line as
/// `solana-sdk` already, so it needs no such conversion.
fn to_sdk_pubkey(address: solana_pubkey_v3::Pubkey) -> Pubkey {
    Pubkey::from(address.to_bytes())
}

fn to_orca_pubkey(pubkey: &Pubkey) -> solana_pubkey_v3::Pubkey {
    solana_pubkey_v3::Pubkey::from(pubkey.to_bytes())
}

/// Rebuild a `solana-instruction` v3 `Instruction` (returned by
/// `orca_whirlpools_client`'s and `spl-associated-token-account-interface`'s
/// generated builders) as the v2 type the rest of this workspace uses.
/// Structurally identical (`program_id`/`accounts`/`data`), just a
/// different major-version crate instance per pubkey/account.
fn to_sdk_instruction(ix: solana_instruction_v3::Instruction) -> Instruction {
    Instruction {
        program_id: to_sdk_pubkey(ix.program_id),
        accounts: ix
            .accounts
            .into_iter()
            .map(|meta| AccountMeta {
                pubkey: to_sdk_pubkey(meta.pubkey),
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            })
            .collect(),
        data: ix.data,
    }
}

/// The classic SPL Token program -- see the module doc's note on the
/// Token-2022 assumption.
fn token_program_v3() -> solana_pubkey_v3::Pubkey {
    solana_pubkey_v3::Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
        .expect("TOKEN_PROGRAM_ID is a valid base58 pubkey")
}

/// The SPL Memo program, required (but unused beyond being referenced) by
/// `SwapV2`'s account list.
fn memo_program_v3() -> solana_pubkey_v3::Pubkey {
    solana_pubkey_v3::Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")
        .expect("memo program id is a valid base58 pubkey")
}

pub struct OrcaClient {
    rpc: Arc<SolanaRpcClient>,
    program_id: Pubkey,
    pools: RwLock<HashMap<(Pubkey, Pubkey), Pubkey>>,
}

impl OrcaClient {
    pub fn new(rpc: Arc<SolanaRpcClient>) -> Self {
        OrcaClient {
            rpc,
            program_id: to_sdk_pubkey(orca_whirlpools_client::ID),
            pools: RwLock::new(HashMap::new()),
        }
    }

    pub fn register_pool(&self, mint_a: Pubkey, mint_b: Pubkey, pool: Pubkey) {
        if let Ok(mut pools) = self.pools.write() {
            pools.insert((mint_a, mint_b), pool);
            pools.insert((mint_b, mint_a), pool);
        }
    }

    fn find_pool(&self, mint_a: &Pubkey, mint_b: &Pubkey) -> Option<Pubkey> {
        self.pools.read().ok()?.get(&(*mint_a, *mint_b)).copied()
    }

    async fn fetch_pool(&self, pool_address: &Pubkey) -> DexResult<Whirlpool> {
        let account = self
            .rpc
            .get_account(pool_address)
            .await
            .map_err(|e| DexError::AccountQuery(e.to_string()))?;
        let data = account.data.ok_or_else(|| {
            DexError::InvalidPoolState(format!("pool {pool_address} has no data"))
        })?;

        Whirlpool::from_bytes(&data)
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

    /// Addresses of the three tick arrays surrounding the pool's current
    /// price: the array containing the current tick, and its immediate
    /// neighbors. These are always the three accounts `SwapV2` needs,
    /// independent of whether each one happens to be initialized on-chain
    /// yet -- unlike [`Self::fetch_surrounding_tick_arrays`], which omits
    /// uninitialized ones for quoting purposes.
    fn tick_array_addresses(pool_address: &Pubkey, pool: &Whirlpool) -> DexResult<[Pubkey; 3]> {
        let span = TICK_ARRAY_SIZE as i32 * pool.tick_spacing as i32;
        let current_start =
            get_tick_array_start_tick_index(pool.tick_current_index, pool.tick_spacing);
        let starts = [current_start - span, current_start, current_start + span];

        let orca_pool_address = to_orca_pubkey(pool_address);
        let mut addresses = [Pubkey::default(); 3];
        for (i, start) in starts.into_iter().enumerate() {
            let (address, _bump) = get_tick_array_address(&orca_pool_address, start, None)
                .map_err(|e| DexError::InvalidPoolState(format!("tick array PDA: {e}")))?;
            addresses[i] = to_sdk_pubkey(address);
        }
        Ok(addresses)
    }

    /// Fetch the (up to) three tick arrays surrounding the pool's current
    /// price. Arrays that haven't been initialized on-chain (no account
    /// exists) are simply omitted rather than erroring, since a swap that
    /// never needs them will still quote correctly.
    async fn fetch_surrounding_tick_arrays(
        &self,
        pool_address: &Pubkey,
        pool: &Whirlpool,
    ) -> DexResult<TickArrays> {
        let addresses = Self::tick_array_addresses(pool_address, pool)?;

        let result = self
            .rpc
            .get_multiple_accounts(&addresses)
            .await
            .map_err(|e| DexError::AccountQuery(e.to_string()))?;

        let mut by_address: HashMap<Pubkey, Vec<u8>> = HashMap::new();
        for info in result.accounts {
            if let Some(data) = info.data {
                by_address.insert(info.address, data);
            }
        }

        let mut facades: Vec<TickArrayFacade> = Vec::with_capacity(3);
        for address in &addresses {
            if let Some(data) = by_address.get(address) {
                let tick_array = orca_whirlpools_client::TickArray::from_bytes(data)
                    .map_err(|e| DexError::InvalidPoolState(format!("tick array decode: {e}")))?;
                let fixed: orca_whirlpools_client::FixedTickArray = tick_array.into();
                facades.push(fixed.into());
            }
        }

        match facades.len() {
            0 => Err(DexError::NoQuote),
            1 => Ok(TickArrays::One(facades[0])),
            2 => Ok(TickArrays::Two(facades[0], facades[1])),
            _ => Ok(TickArrays::Three(facades[0], facades[1], facades[2])),
        }
    }

    fn swap_direction(pool: &Whirlpool, request: &QuoteRequest) -> DexResult<(bool, bool)> {
        let mint_a = to_sdk_pubkey(pool.token_mint_a);
        let mint_b = to_sdk_pubkey(pool.token_mint_b);

        if request.input_mint == mint_a && request.output_mint == mint_b {
            Ok((true, true))
        } else if request.input_mint == mint_b && request.output_mint == mint_a {
            Ok((false, false))
        } else {
            Err(DexError::InvalidPoolState(
                "requested mints do not match pool's token_mint_a/b".to_string(),
            ))
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[async_trait]
impl DexClient for OrcaClient {
    async fn get_quote(&self, request: &QuoteRequest) -> DexResult<Quote> {
        let pool_address = self
            .find_pool(&request.input_mint, &request.output_mint)
            .ok_or(DexError::NoRoute)?;

        let pool = self.fetch_pool(&pool_address).await?;
        let (a_to_b, specified_token_a) = Self::swap_direction(&pool, request)?;
        let tick_arrays = self
            .fetch_surrounding_tick_arrays(&pool_address, &pool)
            .await?;

        let slippage_bps = request.slippage_bps.min(u16::MAX as u32) as u16;
        let quote = swap_quote_by_input_token(
            request.amount,
            specified_token_a,
            slippage_bps,
            pool.clone().into(),
            None,
            tick_arrays,
            now_unix(),
            None,
            None,
        )
        .map_err(|e| DexError::InvalidPoolState(format!("swap quote computation failed: {e:?}")))?;

        let output_vault = if a_to_b {
            pool.token_vault_b
        } else {
            pool.token_vault_a
        };
        let liquidity = self.vault_balance(&to_sdk_pubkey(output_vault)).await?;

        let sqrt_price_f = pool.sqrt_price as f64 / (2f64.powi(64));
        let spot_price_b_per_a = sqrt_price_f * sqrt_price_f;
        let spot_price = if a_to_b {
            spot_price_b_per_a
        } else if spot_price_b_per_a > 0.0 {
            1.0 / spot_price_b_per_a
        } else {
            0.0
        };
        let execution_price = if quote.token_in == 0 {
            0.0
        } else {
            quote.token_est_out as f64 / quote.token_in as f64
        };
        let price_impact = if spot_price > 0.0 {
            ((spot_price - execution_price) / spot_price).max(0.0)
        } else {
            0.0
        };

        Ok(Quote {
            in_amount: quote.token_in,
            out_amount: quote.token_est_out,
            fee_amount: quote.trade_fee,
            fee_bps: (pool.fee_rate as u32 / 100).min(10_000),
            price_impact,
            liquidity,
            route: vec![RouteSegment {
                dex: "Orca".to_string(),
                input_mint: request.input_mint,
                output_mint: request.output_mint,
                input_amount: quote.token_in,
                output_amount: quote.token_est_out,
            }],
            timestamp: Utc::now(),
        })
    }

    async fn get_orderbook(&self, _market: &Pubkey) -> DexResult<solstice_core::types::OrderBook> {
        // Concentrated liquidity has no discrete orderbook.
        Err(DexError::NoQuote)
    }

    async fn get_liquidity(&self, market: &Pubkey) -> DexResult<Liquidity> {
        let pool = self.fetch_pool(market).await?;
        let vault_a = self
            .vault_balance(&to_sdk_pubkey(pool.token_vault_a))
            .await?;
        let vault_b = self
            .vault_balance(&to_sdk_pubkey(pool.token_vault_b))
            .await?;

        Ok(Liquidity {
            base_reserve: vault_a,
            quote_reserve: vault_b,
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
        let request = QuoteRequest::new(
            swap.input_mint,
            swap.output_mint,
            swap.amount,
            swap.slippage_bps,
        );
        let (_a_to_b, specified_token_a) = Self::swap_direction(&pool, &request)?;
        let a_to_b = swap.input_mint == to_sdk_pubkey(pool.token_mint_a);

        // Fresh quote computed here rather than reusing the caller's
        // `Quote`: `Quote` doesn't carry the slippage-protected minimum
        // output or the tick-array facades this needs, and re-deriving it
        // right before building keeps the instruction's numbers as close
        // to submission time as possible.
        let tick_arrays = self
            .fetch_surrounding_tick_arrays(&pool_address, &pool)
            .await?;
        let slippage_bps = swap.slippage_bps.min(u16::MAX as u32) as u16;
        let fresh_quote = swap_quote_by_input_token(
            swap.amount,
            specified_token_a,
            slippage_bps,
            pool.clone().into(),
            None,
            tick_arrays,
            now_unix(),
            None,
            None,
        )
        .map_err(|e| DexError::InvalidPoolState(format!("swap quote computation failed: {e:?}")))?;

        let payer = to_orca_pubkey(&swap.payer);
        let token_program = token_program_v3();
        let token_owner_account_a = get_associated_token_address(&payer, &pool.token_mint_a);
        let token_owner_account_b = get_associated_token_address(&payer, &pool.token_mint_b);

        let mut instructions = vec![
            to_sdk_instruction(create_associated_token_account_idempotent(
                &payer,
                &payer,
                &pool.token_mint_a,
                &token_program,
            )),
            to_sdk_instruction(create_associated_token_account_idempotent(
                &payer,
                &payer,
                &pool.token_mint_b,
                &token_program,
            )),
        ];

        let tick_array_addrs = Self::tick_array_addresses(&pool_address, &pool)?;
        let (oracle, _bump) = get_oracle_address(&to_orca_pubkey(&pool_address), None)
            .map_err(|e| DexError::InvalidPoolState(format!("oracle PDA: {e:?}")))?;

        let swap_accounts = SwapV2 {
            token_program_a: token_program,
            token_program_b: token_program,
            memo_program: memo_program_v3(),
            token_authority: payer,
            whirlpool: to_orca_pubkey(&pool_address),
            token_mint_a: pool.token_mint_a,
            token_mint_b: pool.token_mint_b,
            token_owner_account_a,
            token_vault_a: pool.token_vault_a,
            token_owner_account_b,
            token_vault_b: pool.token_vault_b,
            tick_array0: to_orca_pubkey(&tick_array_addrs[0]),
            tick_array1: to_orca_pubkey(&tick_array_addrs[1]),
            tick_array2: to_orca_pubkey(&tick_array_addrs[2]),
            oracle,
        };
        let args = SwapV2InstructionArgs {
            amount: fresh_quote.token_in,
            other_amount_threshold: fresh_quote.token_min_out,
            sqrt_price_limit: 0,
            amount_specified_is_input: true,
            a_to_b,
            remaining_accounts_info: None,
        };
        instructions.push(to_sdk_instruction(swap_accounts.instruction(args)));

        Ok(SwapInstructions {
            instructions,
            address_lookup_tables: Vec::new(),
        })
    }

    async fn subscribe_prices(&self, _markets: &[Pubkey]) -> mpsc::Receiver<PriceUpdate> {
        let (_tx, rx) = mpsc::channel(1);
        rx
    }

    fn protocol_name(&self) -> &str {
        "Orca"
    }

    fn program_id(&self) -> &Pubkey {
        &self.program_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc() -> Arc<SolanaRpcClient> {
        Arc::new(
            SolanaRpcClient::with_endpoints(vec!["http://localhost:8899".to_string()]).unwrap(),
        )
    }

    #[test]
    fn test_register_and_find_pool() {
        let client = OrcaClient::new(rpc());
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
        let client = OrcaClient::new(rpc());
        assert_eq!(client.protocol_name(), "Orca");
        assert_eq!(
            *client.program_id(),
            to_sdk_pubkey(orca_whirlpools_client::ID)
        );
    }

    #[test]
    fn test_now_unix_is_reasonable() {
        // Sanity bound: after 2024-01-01, well before any plausible clock bug.
        assert!(now_unix() > 1_700_000_000);
    }
}
