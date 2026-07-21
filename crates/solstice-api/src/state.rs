//! Shared application state, threaded through every Axum handler.

use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::SolanaRpcClient;
use solstice_simulation::PaperTradingEngine;
use std::sync::Arc;

/// A wallet's public address and the RPC client to check its balance
/// with. Deliberately holds only the public key — never a private key or
/// signing capability — since the API server has no business being able
/// to sign or move funds; it only reports what it can read.
#[derive(Clone)]
pub struct WalletState {
    pub pubkey: Pubkey,
    pub rpc: Arc<SolanaRpcClient>,
}

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<PaperTradingEngine>,
    pub wallet: Option<WalletState>,
}

impl AppState {
    pub fn new(engine: Arc<PaperTradingEngine>, wallet: Option<WalletState>) -> Self {
        AppState { engine, wallet }
    }
}
