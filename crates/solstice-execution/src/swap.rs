//! Assembling a signed, submittable transaction from a DEX's swap
//! instructions.
//!
//! `solstice_dex::DexClient::build_swap_instructions` (implemented for
//! real by `JupiterClient`, against Jupiter's live `/swap-instructions`
//! API) returns raw `Instruction`s, but nothing previously turned those
//! into an actual `Transaction` ready to sign and submit. This module is
//! that missing link — paired with `solstice_blockchain::SolanaRpcClient`'s
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
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use solstice_blockchain::transaction::TransactionBuilder;
use solstice_blockchain::SolanaRpcClient;
use solstice_dex::{DexClient, Quote, SwapRequest};
use std::time::Duration;

/// Solana's maximum transaction size (legacy or versioned), in bytes.
pub const MAX_TRANSACTION_SIZE: usize = 1232;

/// Fetch swap instructions from `dex` for `swap`/`quote`, assemble them
/// into a legacy `Transaction` against `recent_blockhash`, and sign it
/// with `payer`.
///
/// Takes a concrete `&Keypair` rather than `&dyn Signer`, deliberately:
/// `dyn Signer` isn't `Sync`, and holding a `&dyn Signer` across this
/// function's internal `.await` (fetching instructions) makes the
/// resulting future `!Send` -- which breaks `tokio::spawn`ing anything
/// that calls this (as `LiveTradingEngine::run` does). A concrete
/// `Keypair` is `Send + Sync`, so this has no such problem.
///
/// This deliberately does not build a `VersionedTransaction` with address
/// lookup tables: `DexClient::build_swap_instructions`'s return type
/// (`Vec<Instruction>`) doesn't carry whether a route needs them, so
/// rather than silently assembling something that might not fit on-chain,
/// this checks the assembled transaction's real serialized size and
/// returns `TransactionBuildFailed` if it exceeds the network limit. A
/// caller that hits this needs ALT support, which isn't built here yet.
pub async fn build_swap_transaction(
    dex: &dyn DexClient,
    swap: &SwapRequest,
    quote: &Quote,
    recent_blockhash: Hash,
    payer: &Keypair,
) -> ExecutionResult<Transaction> {
    let instructions = dex.build_swap_instructions(swap, quote).await?;
    if instructions.is_empty() {
        return Err(ExecutionError::TransactionBuildFailed(
            "DEX returned no swap instructions".to_string(),
        ));
    }

    let transaction = TransactionBuilder::new()
        .payer(swap.payer)
        .add_instructions(instructions)
        .build_and_sign(recent_blockhash.to_bytes(), &[payer as &dyn Signer])
        .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;

    let size = bincode::serialize(&transaction)
        .map_err(|e| {
            ExecutionError::TransactionBuildFailed(format!("failed to serialize transaction: {e}"))
        })?
        .len();
    if size > MAX_TRANSACTION_SIZE {
        return Err(ExecutionError::TransactionBuildFailed(format!(
            "assembled swap transaction is {size} bytes, exceeds the {MAX_TRANSACTION_SIZE}-byte \
             network limit -- this route likely needs address lookup tables, which this function \
             does not support"
        )));
    }

    Ok(transaction)
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

    let swap_transaction = build_swap_transaction(dex, swap, quote, blockhash, payer).await?;

    let tip_transaction = match tip {
        Some((tip_account, lamports)) => {
            let instruction = build_tip_instruction(&swap.payer, &tip_account, lamports);
            let tx = TransactionBuilder::new()
                .payer(swap.payer)
                .add_instruction(instruction)
                .build_and_sign(blockhash.to_bytes(), &[payer as &dyn Signer])
                .map_err(|e| ExecutionError::TransactionBuildFailed(e.to_string()))?;
            Some(tx)
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
    use solstice_dex::{DexError, DexResult, Liquidity, PriceUpdate};
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
        ) -> DexResult<Vec<Instruction>> {
            Ok(self.instructions.clone())
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

    #[tokio::test]
    async fn test_build_swap_transaction_signs_successfully() {
        let payer = Keypair::new();
        let dex = MockDex {
            instructions: vec![small_instruction(payer.pubkey())],
        };

        let transaction = build_swap_transaction(
            &dex,
            &sample_swap(payer.pubkey()),
            &sample_quote(),
            Hash::default(),
            &payer,
        )
        .await
        .unwrap();

        assert!(!transaction.signatures.is_empty());
        assert_eq!(transaction.message.account_keys[0], payer.pubkey());
    }

    #[tokio::test]
    async fn test_build_swap_transaction_rejects_empty_instructions() {
        let payer = Keypair::new();
        let dex = MockDex {
            instructions: vec![],
        };

        let result = build_swap_transaction(
            &dex,
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

        let result = build_swap_transaction(
            &dex,
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
}
