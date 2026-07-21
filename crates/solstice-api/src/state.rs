//! Shared application state, threaded through every Axum handler.

use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::SolanaRpcClient;
use solstice_execution::LiveTradingEngine;
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
    /// The automated live-trading engine, if one was configured at
    /// startup. Its own kill switch defaults to disabled regardless of
    /// whether this is `Some` -- configuring it only makes control
    /// endpoints available, it does not itself arm trading.
    pub live: Option<Arc<LiveTradingEngine>>,
}

impl AppState {
    pub fn new(
        engine: Arc<PaperTradingEngine>,
        wallet: Option<WalletState>,
        live: Option<Arc<LiveTradingEngine>>,
    ) -> Self {
        AppState {
            engine,
            wallet,
            live,
        }
    }
}
