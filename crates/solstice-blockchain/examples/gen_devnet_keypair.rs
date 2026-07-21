//! Generate a throwaway keypair for devnet testing (e.g. the
//! `test_sign_submit_confirm_pipeline_on_devnet` dry run). Writes a
//! standard Solana CLI-format keypair JSON file and prints the public
//! address. Devnet-only: the resulting key never holds anything of real
//! value, so writing it to disk in plain form is fine here in a way it
//! never would be for a mainnet key.
//!
//! ```sh
//! cargo run -p solstice-blockchain --example gen_devnet_keypair -- devnet-keypair.json
//! ```

use solana_sdk::signature::{Keypair, Signer};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "devnet-keypair.json".to_string());

    let keypair = Keypair::new();
    let bytes: Vec<u8> = keypair.to_bytes().to_vec();
    std::fs::write(&path, serde_json::to_string(&bytes).unwrap())
        .expect("failed to write keypair file");

    println!("Devnet keypair written to {path}");
    println!("Public address: {}", keypair.pubkey());
    println!();
    println!("Fund it at https://faucet.solana.com (select \"devnet\"), or run:");
    println!(
        "  curl -s https://api.devnet.solana.com -X POST -H \"Content-Type: application/json\" \\"
    );
    println!(
        "    -d '{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"requestAirdrop\",\"params\":[\"{}\",1000000000]}}'",
        keypair.pubkey()
    );
}
