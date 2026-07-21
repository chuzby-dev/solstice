//! Runs the real sign/submit/confirm pipeline against devnet using an
//! already-funded keypair file (see `gen_devnet_keypair`), instead of
//! requesting a fresh airdrop (which this sandbox's IP is rate-limited on).
//!
//! ```sh
//! cargo run -p solstice-blockchain --example devnet_dry_run -- path/to/keypair.json
//! ```

use solana_sdk::signature::{Keypair, Signer};
#[allow(deprecated)]
use solana_sdk::system_instruction;
use solstice_blockchain::transaction::TransactionBuilder;
use solstice_blockchain::SolanaRpcClient;
use std::time::Duration;

const DEVNET_RPC: &str = "https://api.devnet.solana.com";

#[tokio::main]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: devnet_dry_run <keypair.json>");
    let bytes: Vec<u8> =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("failed to read keypair file"))
            .expect("keypair file is not valid JSON byte array");
    let payer = Keypair::try_from(bytes.as_slice()).expect("invalid keypair bytes");

    println!("Using wallet: {}", payer.pubkey());

    let client = SolanaRpcClient::with_endpoints(vec![DEVNET_RPC.to_string()]).unwrap();

    let balance_before = client.get_account(&payer.pubkey()).await;
    println!("Account state before: {balance_before:?}");

    #[allow(deprecated)]
    let instruction = system_instruction::transfer(&payer.pubkey(), &payer.pubkey(), 1);
    let blockhash = client
        .get_latest_blockhash()
        .await
        .expect("failed to fetch blockhash");
    println!("Got recent blockhash: {blockhash}");

    let transaction = TransactionBuilder::new()
        .payer(payer.pubkey())
        .add_instruction(instruction)
        .build_and_sign(blockhash.to_bytes(), &[&payer])
        .expect("failed to build/sign transaction");

    let signature = client
        .send_transaction(&transaction)
        .await
        .expect("failed to submit transaction");
    println!("Submitted! Signature: {signature}");
    println!("Explorer: https://explorer.solana.com/tx/{signature}?cluster=devnet");

    let confirmation = client
        .confirm_transaction(&signature, Duration::from_secs(30), Duration::from_secs(2))
        .await
        .expect("failed to confirm transaction");

    println!("Confirmation: {confirmation:?}");
    if confirmation.is_confirmed() {
        println!("SUCCESS: transaction confirmed on-chain.");
    } else {
        println!("FAILED: transaction did not confirm successfully.");
        std::process::exit(1);
    }
}
