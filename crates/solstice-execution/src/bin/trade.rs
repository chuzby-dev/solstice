//! Manual swap execution CLI. **This can send a real, irreversible
//! on-chain transaction using real funds** if run without `--dry-run`
//! against a funded wallet -- it exists specifically so a human decides,
//! reads the quote, and types an explicit confirmation before that
//! happens. There is no `--yes`/`--force` flag and there will not be one;
//! that's the whole point of this binary versus calling
//! `solstice_execution::execute_swap` directly from code.
//!
//! ```sh
//! cargo run -p solstice-execution --bin trade -- \
//!   --wallet path/to/wallet.json \
//!   --input <input-mint> --output <output-mint> \
//!   --amount <raw-units> --slippage-bps 50 \
//!   [--rpc <url>] [--tip-lamports <n>] [--dry-run]
//! ```
//!
//! `--dry-run` fetches a real quote and builds+signs the real transaction
//! locally (proving it would work) but never submits it -- safe to run
//! against mainnet with zero funds-movement risk.

use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::{SolanaRpcClient, WalletFile};
use solstice_dex::{DexClient, JupiterClient, QuoteRequest, SwapRequest};
use solstice_execution::jito::{JitoClient, JitoConfig};
use solstice_execution::{build_swap_transaction, execute_swap};
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

struct Args {
    wallet_path: PathBuf,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
    slippage_bps: u32,
    rpc_url: String,
    tip_lamports: Option<u64>,
    dry_run: bool,
}

fn print_usage_and_exit() -> ! {
    eprintln!(
        "usage: trade --wallet <path> --input <mint> --output <mint> --amount <raw-units> \
         [--slippage-bps 50] [--rpc <url>] [--tip-lamports <n>] [--dry-run]"
    );
    std::process::exit(1);
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut wallet_path = None;
    let mut input_mint = None;
    let mut output_mint = None;
    let mut amount = None;
    let mut slippage_bps = 50u32;
    let mut rpc_url = None;
    let mut tip_lamports = None;
    let mut dry_run = false;

    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| {
            args.get(i)
                .cloned()
                .unwrap_or_else(|| print_usage_and_exit())
        };
        match args[i].as_str() {
            "--wallet" => {
                wallet_path = Some(PathBuf::from(value(i + 1)));
                i += 2;
            }
            "--input" => {
                input_mint = Some(
                    Pubkey::from_str(&value(i + 1)).unwrap_or_else(|_| print_usage_and_exit()),
                );
                i += 2;
            }
            "--output" => {
                output_mint = Some(
                    Pubkey::from_str(&value(i + 1)).unwrap_or_else(|_| print_usage_and_exit()),
                );
                i += 2;
            }
            "--amount" => {
                amount = Some(
                    value(i + 1)
                        .parse()
                        .unwrap_or_else(|_| print_usage_and_exit()),
                );
                i += 2;
            }
            "--slippage-bps" => {
                slippage_bps = value(i + 1)
                    .parse()
                    .unwrap_or_else(|_| print_usage_and_exit());
                i += 2;
            }
            "--rpc" => {
                rpc_url = Some(value(i + 1));
                i += 2;
            }
            "--tip-lamports" => {
                tip_lamports = Some(
                    value(i + 1)
                        .parse()
                        .unwrap_or_else(|_| print_usage_and_exit()),
                );
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            _ => print_usage_and_exit(),
        }
    }

    let rpc_url = rpc_url
        .or_else(|| std::env::var("HELIUS_RPC_URL").ok())
        .unwrap_or_else(|| {
            eprintln!("--rpc not given and HELIUS_RPC_URL not set");
            print_usage_and_exit()
        });

    Args {
        wallet_path: wallet_path.unwrap_or_else(|| print_usage_and_exit()),
        input_mint: input_mint.unwrap_or_else(|| print_usage_and_exit()),
        output_mint: output_mint.unwrap_or_else(|| print_usage_and_exit()),
        amount: amount.unwrap_or_else(|| print_usage_and_exit()),
        slippage_bps,
        rpc_url,
        tip_lamports,
        dry_run,
    }
}

/// Blocks on a line of stdin, requiring it to exactly equal `expected`
/// (not just "y"/"yes") before returning `true` -- a throwaway typo should
/// abort, not accidentally confirm sending real funds.
fn read_typed_confirmation(expected: &str) -> bool {
    print!("Type {expected:?} to confirm, or anything else to abort: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    line.trim() == expected
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = parse_args();

    let wallet_file = WalletFile::at(&args.wallet_path);
    if !wallet_file.exists() {
        eprintln!(
            "no wallet file at {} -- generate one first (e.g. cargo run -p solstice-blockchain \
             --example gen_devnet_keypair)",
            args.wallet_path.display()
        );
        std::process::exit(1);
    }
    let payer_pubkey = wallet_file.pubkey().expect("failed to read wallet pubkey");

    let rpc = SolanaRpcClient::with_endpoints(vec![args.rpc_url.clone()])
        .expect("failed to build RPC client");
    let balance = rpc
        .get_balance(&payer_pubkey)
        .await
        .expect("failed to fetch wallet balance");

    println!("Wallet:  {payer_pubkey}");
    println!("Balance: {:.6} SOL", balance as f64 / 1_000_000_000.0);
    println!();

    let dex = JupiterClient::new().expect("failed to build Jupiter client");
    let quote_request = QuoteRequest::new(
        args.input_mint,
        args.output_mint,
        args.amount,
        args.slippage_bps,
    );
    println!("Fetching quote...");
    let quote = dex
        .get_quote(&quote_request)
        .await
        .expect("failed to fetch quote");

    let min_out = quote.min_out_amount(args.slippage_bps);
    println!("Route: {} hop(s)", quote.route.len());
    for segment in &quote.route {
        println!(
            "  {} — {} -> {}",
            segment.dex, segment.input_mint, segment.output_mint
        );
    }
    println!(
        "In:      {} (raw units, mint {})",
        quote.in_amount, args.input_mint
    );
    println!(
        "Out:     {} (raw units, mint {})",
        quote.out_amount, args.output_mint
    );
    println!(
        "Min out: {min_out} (at {}bps slippage tolerance)",
        args.slippage_bps
    );
    println!("Price impact: {:.4}%", quote.price_impact * 100.0);
    println!();

    let swap = SwapRequest {
        input_mint: args.input_mint,
        output_mint: args.output_mint,
        amount: args.amount,
        payer: payer_pubkey,
        slippage_bps: args.slippage_bps,
    };

    if args.dry_run {
        println!("--dry-run: building and signing locally, will NOT submit.");
        let keypair = wallet_file.load_keypair().expect("failed to load keypair");
        let blockhash = rpc
            .get_latest_blockhash()
            .await
            .expect("failed to fetch blockhash");
        let transaction = build_swap_transaction(&dex, &rpc, &swap, &quote, blockhash, &keypair)
            .await
            .expect("failed to build swap transaction");
        let size = bincode::serialize(&transaction).unwrap().len();
        println!(
            "Built and signed a {size}-byte transaction with {} signature(s). Not submitted.",
            transaction.signatures.len()
        );
        return;
    }

    println!("################################################################");
    println!("# THIS WILL SEND A REAL, IRREVERSIBLE ON-CHAIN TRANSACTION.");
    println!("# Wallet:  {payer_pubkey}");
    println!(
        "# Sending: {} raw units of {}",
        args.amount, args.input_mint
    );
    println!(
        "# For:     ~{} raw units of {} (min {min_out})",
        quote.out_amount, args.output_mint
    );
    println!("################################################################");
    if !read_typed_confirmation("SEND") {
        println!("Aborted -- no confirmation received.");
        return;
    }

    let keypair = wallet_file.load_keypair().expect("failed to load keypair");
    let jito = JitoClient::new(JitoConfig::default()).expect("failed to build Jito client");
    // Best-effort: if a tip was requested but fetching tip accounts fails,
    // fall through to a tip-less submission (still tries Jito, then direct
    // RPC via `submit_with_fallback`) rather than aborting the trade.
    let tip = match args.tip_lamports {
        Some(lamports) => match jito.get_tip_accounts().await {
            Ok(accounts) if !accounts.is_empty() => Some((accounts[0], lamports)),
            _ => {
                eprintln!("warning: failed to fetch Jito tip accounts, submitting without a tip");
                None
            }
        },
        None => None,
    };

    println!("Submitting...");
    let outcome = execute_swap(
        &jito,
        &rpc,
        &dex,
        &swap,
        &quote,
        &keypair,
        tip,
        Duration::from_secs(60),
        Duration::from_secs(2),
    )
    .await
    .expect("swap execution failed");

    println!("Method: {:?}", outcome.method);
    if let Some(bundle_id) = &outcome.bundle_id {
        println!("Jito bundle id: {bundle_id}");
    }
    for signature in &outcome.signatures {
        println!("Signature: {signature}");
        println!("Explorer:  https://explorer.solana.com/tx/{signature}");
    }
}
