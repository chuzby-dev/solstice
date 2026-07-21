//! Shared application state, threaded through every Axum handler.

use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::{SolanaRpcClient, WalletFile};
use solstice_dex::JupiterClient;
use solstice_execution::jito::JitoClient;
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

/// Everything needed to execute a real, user-initiated SOL<->USDC
/// conversion from the Wallet page's manual "Convert" action. Unlike
/// `WalletState`, this *can* sign and submit -- but like
/// `LiveTradingEngine`, it never holds the private key at rest: `WalletFile`
/// only reads the keypair from disk transiently, at the moment a convert
/// request is actually being signed, and only in response to a request
/// this server received (never on a timer, never without one).
pub struct ConvertState {
    pub wallet_file: WalletFile,
    pub wallet_pubkey: Pubkey,
    pub rpc: Arc<SolanaRpcClient>,
    pub jito: JitoClient,
    pub dex: JupiterClient,
    pub sol_mint: Pubkey,
    pub sol_decimals: u8,
    pub usdc_mint: Pubkey,
    pub usdc_decimals: u8,
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
    /// Manual convert support, if a wallet was configured at startup.
    /// Wrapped in `Arc` (rather than deriving `Clone` on the inner
    /// clients) purely so `AppState` itself stays cheaply cloneable for
    /// Axum's `State` extractor.
    pub convert: Option<Arc<ConvertState>>,
}

impl AppState {
    pub fn new(
        engine: Arc<PaperTradingEngine>,
        wallet: Option<WalletState>,
        live: Option<Arc<LiveTradingEngine>>,
        convert: Option<Arc<ConvertState>>,
    ) -> Self {
        AppState {
            engine,
            wallet,
            live,
            convert,
        }
    }
}
