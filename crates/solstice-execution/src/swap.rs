//! Assembling a signed, submittable transaction from a DEX's swap
//! instructions.
//!
//! `solstice_dex::DexClient::build_swap_instructions` (implemented for
//! real by `JupiterClient`, against Jupiter's live `/swap-instructions`
//! API) returns raw instructions plus any address lookup tables (ALTs) the
//! route needs, but nothing previously turned those into an actual
//! transaction ready to sign and submit. This module is that missing link
//! — paired with `solstice_blockchain::SolanaRpcClient`'s
//! `send_transaction`/`confirm_transaction` and
//! `solstice_execution::jito`'s bundle submission, it completes the chain
//! from "here are the instructions" to "here's what happened on-chain."
//!
//! It does not decide *when* to trade, size a position, or run risk
//! checks — those stay in `PositionSizer`/`PreTradeRiskChecker` upstream of
//! this. It also does not hold, generate, or manage a signing key: callers
//! supply their own `&dyn Signer`.

use crate::error::{ExecutionError, ExecutionResult};
use crate::jito::{build_tip_instruction, submit_with_fallback, JitoClient, SubmissionOutcome};
use solana_sdk::address_lookup_table::state::AddressLookupTable;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::message::{v0, AddressLookupTableAccount, VersionedMessage};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;
use solstice_blockchain::transaction::TransactionBuilder;
use solstice_blockchain::SolanaRpcClient;
use solstice_dex::{DexClient, Quote, SwapRequest};
use std::time::Duration;

/// Solana's maximum transaction size (legacy or versioned), in bytes.
pub const MAX_TRANSACTION_SIZE: usize = 1232;

/// Fetch `table` from `rpc` and deserialize it into the form
/// `v0::Message::try_compile` needs.
async fn fetch_lookup_table_account(
    rpc: &SolanaRpcClient,
    table: Pubkey,
) -> ExecutionResult<AddressLookupTableAccount> {
    let account = rpc.get_account(&table).await.map_err(|e| {
        ExecutionError::TransactionBuildFailed(format!(
            "failed to fetch address lookup table {table}: {e}"
        ))
    })?;
    let data = account.data.ok_or_else(|| {
        ExecutionError::TransactionBuildFailed(format!(
            "address lookup table {table} has no account data"
        ))
    })?;
    let parsed = AddressLookupTable::deserialize(&data).map_err(|e| {
        ExecutionError::TransactionBuildFailed(format!(
            "failed to deserialize address lookup table {table}: {e}"
        ))
    })?;
    Ok(AddressLookupTableAccount {
        key: table,
        addresses: parsed.addresses.into_owned(),
    })
}

/// Two instructions are equal for dedup purposes if they'd have the
/// exact same on-chain effect: same program, same data, same accounts in
/// the same order with the same signer/writable flags. Written by hand
/// (rather than deriving/relying on `Instruction: PartialEq`) so this
/// doesn't depend on that trait existing upstream.
fn instructions_equal(a: &Instruction, b: &Instruction) -> bool {
    a.program_id == b.program_id
        && a.data == b.data
        && a.accounts.len() == b.accounts.len()
        && a.accounts.iter().zip(b.accounts.iter()).all(|(x, y)| {
            x.pubkey == y.pubkey && x.is_signer == y.is_signer && x.is_writable == y.is_writable
        })
}

/// Drop exact-duplicate instructions, keeping each one's first
/// occurrence and preserving order otherwise. Every `DexClient` in this
/// workspace prepends an idempotent ATA-creation instruction for each
/// mint it touches; when two legs of an atomic transaction share a mint
/// (the common case -- the sell leg's input is the buy leg's output),
/// each emits its own byte-identical copy. Removing the duplicate is
/// always safe (the instruction is a no-op if the account already
/// exists) and buys back real bytes against the 1232-byte transaction
/// limit -- confirmed necessary against a live RAY/USDC atomic attempt
/// that came in at 1274 bytes, over the limit even with 3 ALTs applied.
fn dedup_instructions(instructions: Vec<Instruction>) -> Vec<Instruction> {
    let mut deduped: Vec<Instruction> = Vec::with_capacity(instructions.len());
    for instruction in instructions {
        if !deduped
            .iter()
            .any(|kept| instructions_equal(kept, &instruction))
        {
            deduped.push(instruction);
        }
    }
    deduped
}

/// Fetch swap instructions from `dex` for `swap`/`quote` and assemble them
/// into a signed transaction against `recent_blockhash`, signed with
/// `payer`.
///
/// If the route needs no address lookup tables, this builds a legacy
/// message (wrapped as a `VersionedTransaction` for a uniform return type
/// down the submission pipeline). If it does, `rpc` is used to fetch and
/// deserialize each table so a versioned `v0` message can be compiled
/// against them — this is what lets routes that don't fit in the legacy
/// 1232-byte limit actually submit, instead of being rejected outright.
///
/// Takes a concrete `&Keypair` rather than `&dyn Signer`, deliberately:
/// `dyn Signer` isn't `Sync`, and holding a `&dyn Signer` across this
/// function's internal `.await` (fetching instructions) makes the
/// resulting future `!Send` -- which breaks `tokio::spawn`ing anything
/// that calls this (as `LiveTradingEngine::run` does). A concrete
/// `Keypair` is `Send + Sync`, so this has no such problem.
pub async fn build_swap_transaction(
    dex: &dyn DexClient,
    rpc: &SolanaRpcClient,
    swap: &SwapRequest,
    quote: &Quote,
    recent_blockhash: Hash,
    payer: &Keypair,
) -> ExecutionResult<VersionedTransaction> {
    let swap_instructions = dex.build_swap_instructions(swap, quote).await?;
    if swap_instructions.instructions.is_empty() {
        return Err(ExecutionError::TransactionBuildFailed(
            "DEX returned no swap instructions".to_string(),
        ));
    }

    let lookup_table_accounts = if swap_instructions.address_lookup_tables.is_empty() {
        Vec::new()
    } else {
        let mut accounts = Vec::with_capacity(swap_instructions.address_lookup_tables.len());
        for table in &swap_instructions.address_lookup_tables {
            accounts.push(fetch_lookup_table_account(rpc, *table).await?);
        }
        accounts
    };

    let message = v0::Message::try_compile(
        &swap.payer,
        &swap_instructions.instructions,
        &lookup_table_accounts,
        recent_blockhash,
    )
    .map_err(|e| {
        ExecutionError::TransactionBuildFailed(format!("failed to compile v0 message: {e}"))
    })?;

    let transaction = VersionedTransaction::try_new(VersionedMessage::V0(message), &[payer])
        .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;

    let size = bincode::serialize(&transaction)
        .map_err(|e| {
            ExecutionError::TransactionBuildFailed(format!("failed to serialize transaction: {e}"))
        })?
        .len();
    if size > MAX_TRANSACTION_SIZE {
        return Err(ExecutionError::TransactionBuildFailed(format!(
            "assembled swap transaction is {size} bytes, exceeds the {MAX_TRANSACTION_SIZE}-byte \
             network limit even with {} address lookup table(s) applied",
            lookup_table_accounts.len()
        )));
    }

    Ok(transaction)
}

/// Fetch instructions for every `(dex, swap, quote)` leg and concatenate
/// them into a single signed transaction, so all legs land together or
/// the whole thing reverts -- e.g. buy on DEX A then sell on DEX B for a
/// cross-DEX arbitrage, with no window between legs for the market to
/// move against the position the way two separate transactions has.
///
/// Legs are appended in order (`legs[0]`'s instructions first). This
/// relies on two properties that hold for every `DexClient` in this
/// workspace: (1) each ATA-creation instruction is idempotent
/// (`create_associated_token_account_idempotent`), so composing legs
/// that touch the same token account is safe, and (2) at most one leg
/// can be Jupiter (the only client that emits `ComputeBudget`
/// instructions) since `find_arb_opportunity` never pairs a DEX with
/// itself -- so there's no risk of duplicate compute-budget instructions
/// colliding. If a leg's baked-in input amount turns out to exceed what
/// an earlier leg actually produced (e.g. the buy underperformed its
/// quote), Solana's preflight simulation -- on by default for
/// `SolanaRpcClient::send_transaction` -- rejects the whole transaction
/// before anything lands or any fee beyond the attempt is paid.
pub async fn build_atomic_swap_transaction(
    legs: &[(&dyn DexClient, &SwapRequest, &Quote)],
    rpc: &SolanaRpcClient,
    recent_blockhash: Hash,
    payer: &Keypair,
) -> ExecutionResult<VersionedTransaction> {
    let Some((_, first_swap, _)) = legs.first() else {
        return Err(ExecutionError::TransactionBuildFailed(
            "no legs to build an atomic transaction from".to_string(),
        ));
    };
    let payer_pubkey = first_swap.payer;

    let mut instructions = Vec::new();
    let mut lookup_tables: Vec<Pubkey> = Vec::new();
    for (dex, swap, quote) in legs {
        let swap_instructions = dex.build_swap_instructions(swap, quote).await?;
        if swap_instructions.instructions.is_empty() {
            return Err(ExecutionError::TransactionBuildFailed(
                "DEX returned no swap instructions for one leg of an atomic transaction"
                    .to_string(),
            ));
        }
        instructions.extend(swap_instructions.instructions);
        for table in swap_instructions.address_lookup_tables {
            if !lookup_tables.contains(&table) {
                lookup_tables.push(table);
            }
        }
    }

    // Every `DexClient` here prepends an idempotent ATA-creation
    // instruction for each mint it touches -- fine on its own, but two
    // legs sharing a mint (the common case: the sell leg's input is the
    // buy leg's output) each emit their own byte-identical copy. Left in,
    // this was enough to push a real RAY/USDC atomic transaction to 1274
    // bytes, over the 1232-byte limit even with 3 ALTs applied. Since
    // `create_associated_token_account_idempotent` is a no-op if the
    // account already exists, dropping exact-duplicate instructions
    // (keeping the first occurrence) is always safe and buys back exactly
    // the redundant bytes -- never touches a genuinely distinct swap
    // instruction, since no two of those are ever byte-identical.
    let instructions = dedup_instructions(instructions);

    let lookup_table_accounts = if lookup_tables.is_empty() {
        Vec::new()
    } else {
        let mut accounts = Vec::with_capacity(lookup_tables.len());
        for table in &lookup_tables {
            accounts.push(fetch_lookup_table_account(rpc, *table).await?);
        }
        accounts
    };

    let message = v0::Message::try_compile(
        &payer_pubkey,
        &instructions,
        &lookup_table_accounts,
        recent_blockhash,
    )
    .map_err(|e| {
        ExecutionError::TransactionBuildFailed(format!("failed to compile v0 message: {e}"))
    })?;

    let transaction = VersionedTransaction::try_new(VersionedMessage::V0(message), &[payer])
        .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;

    let size = bincode::serialize(&transaction)
        .map_err(|e| {
            ExecutionError::TransactionBuildFailed(format!("failed to serialize transaction: {e}"))
        })?
        .len();
    if size > MAX_TRANSACTION_SIZE {
        return Err(ExecutionError::TransactionBuildFailed(format!(
            "assembled atomic arb transaction is {size} bytes, exceeds the {MAX_TRANSACTION_SIZE}-byte \
             network limit even with {} address lookup table(s) applied",
            lookup_table_accounts.len()
        )));
    }

    Ok(transaction)
}

/// Atomic equivalent of [`execute_swap`]: build every leg into one
/// transaction (see [`build_atomic_swap_transaction`]), then submit it
/// exactly once (Jito bundle first, falling back to direct RPC), and
/// confirm it landed. Either every leg executes or none do.
#[allow(clippy::too_many_arguments)]
pub async fn execute_atomic_arb(
    jito: &JitoClient,
    rpc: &SolanaRpcClient,
    legs: &[(&dyn DexClient, &SwapRequest, &Quote)],
    payer: &Keypair,
    tip: Option<(Pubkey, u64)>,
    confirm_timeout: Duration,
    poll_interval: Duration,
) -> ExecutionResult<SubmissionOutcome> {
    let Some((_, first_swap, _)) = legs.first() else {
        return Err(ExecutionError::TransactionBuildFailed(
            "no legs to execute an atomic arb transaction from".to_string(),
        ));
    };
    let payer_pubkey = first_swap.payer;

    let blockhash = rpc
        .get_latest_blockhash()
        .await
        .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;

    let transaction = build_atomic_swap_transaction(legs, rpc, blockhash, payer).await?;

    let tip_transaction = match tip {
        Some((tip_account, lamports)) => {
            let instruction = build_tip_instruction(&payer_pubkey, &tip_account, lamports);
            let tx = TransactionBuilder::new()
                .payer(payer_pubkey)
                .add_instruction(instruction)
                .build_and_sign(blockhash.to_bytes(), &[payer as &dyn Signer])
                .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;
            Some(VersionedTransaction::from(tx))
        }
        None => None,
    };

    submit_with_fallback(
        jito,
        rpc,
        &[transaction],
        tip_transaction,
        confirm_timeout,
        poll_interval,
    )
    .await
    .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))
}

/// End-to-end swap execution: build, sign, and submit a real transaction
/// (Jito bundle first, falling back to direct RPC — see
/// `crate::jito::submit_with_fallback`), then confirm it landed.
///
/// **This function performs a real, irreversible on-chain action** if
/// pointed at mainnet with a funded wallet — it does not itself contain
/// any confirmation gate or dry-run mode. That's deliberate: this is the
/// reusable core meant to eventually be called from an automated engine,
/// the same way `PaperTradingEngine::act_on_signal` calls into the paper
/// order pipeline. Human-in-the-loop confirmation belongs at the call
/// site — see `solstice-execution`'s `trade` binary, which gates this
/// behind an explicit typed confirmation before ever calling it for real
/// funds. A future automated caller would skip that gate by design, once
/// that's an explicit decision to wire up — not by omission here.
#[allow(clippy::too_many_arguments)]
pub async fn execute_swap(
    jito: &JitoClient,
    rpc: &SolanaRpcClient,
    dex: &dyn DexClient,
    swap: &SwapRequest,
    quote: &Quote,
    payer: &Keypair,
    tip: Option<(Pubkey, u64)>,
    confirm_timeout: Duration,
    poll_interval: Duration,
) -> ExecutionResult<SubmissionOutcome> {
    let blockhash = rpc
        .get_latest_blockhash()
        .await
        .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;

    let swap_transaction = build_swap_transaction(dex, rpc, swap, quote, blockhash, payer).await?;

    let tip_transaction = match tip {
        Some((tip_account, lamports)) => {
            let instruction = build_tip_instruction(&swap.payer, &tip_account, lamports);
            let tx = TransactionBuilder::new()
                .payer(swap.payer)
                .add_instruction(instruction)
                .build_and_sign(blockhash.to_bytes(), &[payer as &dyn Signer])
                .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;
            Some(VersionedTransaction::from(tx))
        }
        None => None,
    };

    submit_with_fallback(
        jito,
        rpc,
        &[swap_transaction],
        tip_transaction,
        confirm_timeout,
        poll_interval,
    )
    .await
    .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use solana_sdk::instruction::{AccountMeta, Instruction};
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::Keypair;
    use solstice_dex::{DexError, DexResult, Liquidity, PriceUpdate, SwapInstructions};
    use tokio::sync::mpsc;

    struct MockDex {
        instructions: Vec<Instruction>,
    }

    #[async_trait]
    impl DexClient for MockDex {
        async fn get_quote(&self, _request: &solstice_dex::QuoteRequest) -> DexResult<Quote> {
            unimplemented!("not needed for these tests")
        }

        async fn get_orderbook(
            &self,
            _market: &Pubkey,
        ) -> DexResult<solstice_core::types::OrderBook> {
            Err(DexError::NoQuote)
        }

        async fn get_liquidity(&self, _market: &Pubkey) -> DexResult<Liquidity> {
            Err(DexError::NoQuote)
        }

        async fn build_swap_instructions(
            &self,
            _swap: &SwapRequest,
            _quote: &Quote,
        ) -> DexResult<SwapInstructions> {
            Ok(SwapInstructions {
                instructions: self.instructions.clone(),
                address_lookup_tables: Vec::new(),
            })
        }

        async fn subscribe_prices(&self, _markets: &[Pubkey]) -> mpsc::Receiver<PriceUpdate> {
            let (_tx, rx) = mpsc::channel(1);
            rx
        }

        fn protocol_name(&self) -> &str {
            "Mock"
        }

        fn program_id(&self) -> &Pubkey {
            static ID: Pubkey = Pubkey::new_from_array([0u8; 32]);
            &ID
        }
    }

    fn sample_quote() -> Quote {
        Quote {
            in_amount: 1_000_000,
            out_amount: 2_000_000,
            fee_amount: 2_500,
            fee_bps: 25,
            price_impact: 0.001,
            liquidity: 10_000_000,
            route: vec![],
            timestamp: chrono::Utc::now(),
        }
    }

    fn sample_swap(payer: Pubkey) -> SwapRequest {
        SwapRequest {
            input_mint: Pubkey::new_unique(),
            output_mint: Pubkey::new_unique(),
            amount: 1_000_000,
            payer,
            slippage_bps: 50,
        }
    }

    fn small_instruction(payer: Pubkey) -> Instruction {
        Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta::new(payer, true)],
            data: vec![1, 2, 3],
        }
    }

    /// RPC pointed at an address nothing listens on. Fine for these tests:
    /// none of them exercise the ALT-fetch path, so `rpc` is never
    /// actually called.
    fn test_rpc() -> SolanaRpcClient {
        SolanaRpcClient::with_endpoints(vec!["http://127.0.0.1:1".to_string()]).unwrap()
    }

    #[tokio::test]
    async fn test_build_swap_transaction_signs_successfully() {
        let payer = Keypair::new();
        let dex = MockDex {
            instructions: vec![small_instruction(payer.pubkey())],
        };
        let rpc = test_rpc();

        let transaction = build_swap_transaction(
            &dex,
            &rpc,
            &sample_swap(payer.pubkey()),
            &sample_quote(),
            Hash::default(),
            &payer,
        )
        .await
        .unwrap();

        assert!(!transaction.signatures.is_empty());
        assert_eq!(transaction.message.static_account_keys()[0], payer.pubkey());
    }

    #[tokio::test]
    async fn test_build_swap_transaction_rejects_empty_instructions() {
        let payer = Keypair::new();
        let dex = MockDex {
            instructions: vec![],
        };
        let rpc = test_rpc();

        let result = build_swap_transaction(
            &dex,
            &rpc,
            &sample_swap(payer.pubkey()),
            &sample_quote(),
            Hash::default(),
            &payer,
        )
        .await;

        assert!(matches!(
            result,
            Err(ExecutionError::TransactionBuildFailed(_))
        ));
    }

    #[tokio::test]
    async fn test_build_swap_transaction_rejects_oversized_result() {
        let payer = Keypair::new();
        // Many instructions with several accounts each -- easily exceeds
        // the 1232-byte legacy transaction limit.
        let instructions: Vec<Instruction> = (0..40)
            .map(|_| Instruction {
                program_id: Pubkey::new_unique(),
                accounts: vec![
                    AccountMeta::new(Pubkey::new_unique(), false),
                    AccountMeta::new(Pubkey::new_unique(), false),
                    AccountMeta::new(Pubkey::new_unique(), false),
                ],
                data: vec![0u8; 32],
            })
            .collect();
        let dex = MockDex { instructions };
        let rpc = test_rpc();

        let result = build_swap_transaction(
            &dex,
            &rpc,
            &sample_swap(payer.pubkey()),
            &sample_quote(),
            Hash::default(),
            &payer,
        )
        .await;

        assert!(matches!(
            result,
            Err(ExecutionError::TransactionBuildFailed(_))
        ));
    }

    #[tokio::test]
    async fn test_build_atomic_swap_transaction_merges_both_legs() {
        let payer = Keypair::new();
        let buy_dex = MockDex {
            instructions: vec![small_instruction(payer.pubkey())],
        };
        let sell_dex = MockDex {
            instructions: vec![small_instruction(payer.pubkey())],
        };
        let rpc = test_rpc();
        let buy_swap = sample_swap(payer.pubkey());
        let sell_swap = sample_swap(payer.pubkey());
        let quote = sample_quote();

        let legs: [(&dyn DexClient, &SwapRequest, &Quote); 2] = [
            (&buy_dex, &buy_swap, &quote),
            (&sell_dex, &sell_swap, &quote),
        ];

        let transaction = build_atomic_swap_transaction(&legs, &rpc, Hash::default(), &payer)
            .await
            .unwrap();

        assert!(!transaction.signatures.is_empty());
        assert_eq!(transaction.message.static_account_keys()[0], payer.pubkey());
        // Both legs' instructions must be present in one transaction --
        // the whole point of atomicity.
        assert_eq!(transaction.message.instructions().len(), 2);
    }

    #[tokio::test]
    async fn test_build_atomic_swap_transaction_dedups_shared_setup_instruction() {
        let payer = Keypair::new();
        // Simulates both legs prepending the exact same idempotent
        // ATA-creation instruction (the common case: the sell leg's
        // input mint is the buy leg's output mint) plus one instruction
        // unique to each leg.
        let shared_setup = small_instruction(payer.pubkey());
        let buy_dex = MockDex {
            instructions: vec![shared_setup.clone(), small_instruction(payer.pubkey())],
        };
        let sell_dex = MockDex {
            instructions: vec![shared_setup, small_instruction(payer.pubkey())],
        };
        let rpc = test_rpc();
        let swap = sample_swap(payer.pubkey());
        let quote = sample_quote();

        let legs: [(&dyn DexClient, &SwapRequest, &Quote); 2] =
            [(&buy_dex, &swap, &quote), (&sell_dex, &swap, &quote)];

        let transaction = build_atomic_swap_transaction(&legs, &rpc, Hash::default(), &payer)
            .await
            .unwrap();

        // 4 instructions went in (2 per leg), but the shared setup
        // instruction should only appear once: 3 total, not 4.
        assert_eq!(transaction.message.instructions().len(), 3);
    }

    #[tokio::test]
    async fn test_build_atomic_swap_transaction_rejects_empty_legs() {
        let payer = Keypair::new();
        let rpc = test_rpc();

        let result = build_atomic_swap_transaction(&[], &rpc, Hash::default(), &payer).await;

        assert!(matches!(
            result,
            Err(ExecutionError::TransactionBuildFailed(_))
        ));
    }

    #[tokio::test]
    async fn test_build_atomic_swap_transaction_rejects_leg_with_no_instructions() {
        let payer = Keypair::new();
        let buy_dex = MockDex {
            instructions: vec![small_instruction(payer.pubkey())],
        };
        let sell_dex = MockDex {
            instructions: vec![],
        };
        let rpc = test_rpc();
        let swap = sample_swap(payer.pubkey());
        let quote = sample_quote();

        let legs: [(&dyn DexClient, &SwapRequest, &Quote); 2] =
            [(&buy_dex, &swap, &quote), (&sell_dex, &swap, &quote)];

        let result = build_atomic_swap_transaction(&legs, &rpc, Hash::default(), &payer).await;

        assert!(matches!(
            result,
            Err(ExecutionError::TransactionBuildFailed(_))
        ));
    }
}
