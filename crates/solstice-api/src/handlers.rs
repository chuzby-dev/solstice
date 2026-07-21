//! REST endpoint handlers.

use crate::dto::{
    ConvertDirection, ConvertRequest, ConvertResponse, DevnetBalanceResponse, LiveConfigRequest,
    PerformanceResponse, PositionsResponse, StatusResponse, TradeResponse, TradesResponse,
    WalletResponse,
};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use solstice_blockchain::SolanaRpcClient;
use solstice_dex::{DexClient, QuoteRequest, SwapRequest};
use solstice_execution::{execute_swap, LiveStatusSnapshot};
use std::time::Duration;

const LAMPORTS_PER_SOL: f64 = 1_000_000_000.0;
const DEVNET_RPC_URL: &str = "https://api.devnet.solana.com";

pub async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let snapshot = state.engine.portfolio_snapshot();
    Json(StatusResponse {
        status: "running",
        monitored_pairs: state.engine.pair_labels(),
        open_positions: snapshot.positions.len(),
        total_value_usd: snapshot.total_value_usd,
        circuit_breaker_tripped: state.engine.circuit_breaker_tripped(),
    })
}

pub async fn positions(State(state): State<AppState>) -> Json<PositionsResponse> {
    Json(state.engine.portfolio_snapshot().into())
}

pub async fn performance(State(state): State<AppState>) -> Json<PerformanceResponse> {
    Json(state.engine.portfolio_snapshot().into())
}

pub async fn trades(State(state): State<AppState>) -> Json<TradesResponse> {
    let orders = state.engine.order_manager().all_orders();
    Json(TradesResponse {
        trades: orders.iter().map(TradeResponse::from).collect(),
    })
}

/// Read-only wallet status: address, mainnet SOL balance, and mainnet
/// USDC balance. `404` if no wallet is configured for this server
/// (`WALLET_KEYPAIR_PATH` unset).
pub async fn wallet(State(state): State<AppState>) -> ApiResult<Json<WalletResponse>> {
    let wallet = state
        .wallet
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("no wallet configured".to_string()))?;

    let balance_lamports = wallet
        .rpc
        .get_balance(&wallet.pubkey)
        .await
        .map_err(|e| ApiError::Upstream(format!("failed to fetch wallet balance: {e}")))?;

    // USDC balance is best-effort: a wallet with no USDC token account
    // yet is a normal state (reports 0), same philosophy as
    // `get_balance` for a never-funded SOL address.
    let usdc_mint = usdc_mint();
    let usdc_balance_raw = wallet
        .rpc
        .get_token_balance(&wallet.pubkey, &usdc_mint)
        .await
        .map_err(|e| ApiError::Upstream(format!("failed to fetch USDC balance: {e}")))?;

    Ok(Json(WalletResponse {
        address: wallet.pubkey.to_string(),
        balance_lamports,
        balance_sol: balance_lamports as f64 / LAMPORTS_PER_SOL,
        usdc_balance_raw,
        usdc_balance: usdc_balance_raw as f64 / 10f64.powi(USDC_DECIMALS as i32),
    }))
}

/// Read-only devnet SOL balance for the same wallet address. Devnet is a
/// separate ledger from mainnet, but the same keypair holds an address on
/// both -- useful for seeing leftover devnet SOL from earlier
/// faucet-funded testing. `404` if no wallet is configured.
pub async fn wallet_devnet(
    State(state): State<AppState>,
) -> ApiResult<Json<DevnetBalanceResponse>> {
    let wallet = state
        .wallet
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("no wallet configured".to_string()))?;

    let devnet_rpc = SolanaRpcClient::with_endpoints(vec![DEVNET_RPC_URL.to_string()])
        .map_err(|e| ApiError::Upstream(format!("failed to build devnet RPC client: {e}")))?;
    let balance_lamports = devnet_rpc
        .get_balance(&wallet.pubkey)
        .await
        .map_err(|e| ApiError::Upstream(format!("failed to fetch devnet balance: {e}")))?;

    Ok(Json(DevnetBalanceResponse {
        address: wallet.pubkey.to_string(),
        balance_lamports,
        balance_sol: balance_lamports as f64 / LAMPORTS_PER_SOL,
    }))
}

const USDC_DECIMALS: u8 = 6;

fn usdc_mint() -> solana_sdk::pubkey::Pubkey {
    use std::str::FromStr;
    solana_sdk::pubkey::Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
        .expect("USDC mint is a valid pubkey")
}

/// **Executes a real, irreversible on-chain swap** converting between the
/// configured wallet's own SOL and USDC, in whichever direction the
/// caller requests. This is not a preview endpoint and there is no
/// separate confirmation step here -- the dashboard's Wallet page gates
/// this behind its own typed confirmation before ever calling it, the
/// same "a typo should abort, not confirm" pattern as the live trading
/// kill switch and the `trade` CLI's `SEND` gate. `404` if no wallet is
/// configured.
pub async fn wallet_convert(
    State(state): State<AppState>,
    Json(body): Json<ConvertRequest>,
) -> ApiResult<Json<ConvertResponse>> {
    let convert = state
        .convert
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("no wallet configured".to_string()))?;

    if !body.amount.is_finite() || body.amount <= 0.0 {
        return Err(ApiError::BadRequest(
            "amount must be a positive finite number".to_string(),
        ));
    }
    let slippage_bps = body.slippage_bps.unwrap_or(150);

    let (input_mint, input_decimals, output_mint) = match body.direction {
        ConvertDirection::SolToUsdc => (convert.sol_mint, convert.sol_decimals, convert.usdc_mint),
        ConvertDirection::UsdcToSol => (convert.usdc_mint, convert.usdc_decimals, convert.sol_mint),
    };
    let amount_raw = (body.amount * 10f64.powi(input_decimals as i32)).round() as u64;
    if amount_raw == 0 {
        return Err(ApiError::BadRequest(
            "amount is too small to convert to a nonzero raw token amount".to_string(),
        ));
    }

    let swap = SwapRequest {
        input_mint,
        output_mint,
        amount: amount_raw,
        payer: convert.wallet_pubkey,
        slippage_bps,
    };

    let quote = convert
        .dex
        .get_quote(&QuoteRequest::new(
            input_mint,
            output_mint,
            amount_raw,
            slippage_bps,
        ))
        .await
        .map_err(|e| ApiError::Upstream(format!("failed to fetch conversion quote: {e}")))?;

    let keypair = convert
        .wallet_file
        .load_keypair()
        .map_err(|e| ApiError::Upstream(format!("failed to load wallet key: {e}")))?;

    let outcome = execute_swap(
        &convert.jito,
        &convert.rpc,
        &convert.dex,
        &swap,
        &quote,
        &keypair,
        None,
        Duration::from_secs(60),
        Duration::from_secs(2),
    )
    .await
    .map_err(|e| ApiError::Upstream(format!("conversion failed: {e}")))?;

    let output_decimals = match body.direction {
        ConvertDirection::SolToUsdc => convert.usdc_decimals,
        ConvertDirection::UsdcToSol => convert.sol_decimals,
    };

    Ok(Json(ConvertResponse {
        method: format!("{:?}", outcome.method),
        signatures: outcome.signatures.iter().map(|s| s.to_string()).collect(),
        input_amount: body.amount,
        output_amount: quote.out_amount as f64 / 10f64.powi(output_decimals as i32),
    }))
}

fn require_live(
    state: &AppState,
) -> ApiResult<&std::sync::Arc<solstice_execution::LiveTradingEngine>> {
    state
        .live
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("no live trading engine configured".to_string()))
}

/// Live-engine status: kill-switch state, capital cap/deployed/available,
/// and open positions.
pub async fn live_status(State(state): State<AppState>) -> ApiResult<Json<LiveStatusSnapshot>> {
    Ok(Json(require_live(&state)?.status()))
}

/// Arms the live engine: from the next tick onward, approved signals
/// submit real transactions. The dashboard is expected to gate this
/// behind its own explicit confirmation -- this endpoint itself performs
/// no trade, it only flips the switch.
pub async fn live_enable(State(state): State<AppState>) -> ApiResult<Json<LiveStatusSnapshot>> {
    let live = require_live(&state)?;
    live.enable();
    Ok(Json(live.status()))
}

/// Disarms the live engine. Always safe, always available, no
/// confirmation needed -- turning trading off should never be blocked.
pub async fn live_disable(State(state): State<AppState>) -> ApiResult<Json<LiveStatusSnapshot>> {
    let live = require_live(&state)?;
    live.disable();
    Ok(Json(live.status()))
}

/// Adjusts the hard capital ceiling and/or the minimum-confidence-to-act
/// threshold the live engine uses. Either field may be omitted to leave
/// it unchanged.
pub async fn live_set_config(
    State(state): State<AppState>,
    Json(body): Json<LiveConfigRequest>,
) -> ApiResult<Json<LiveStatusSnapshot>> {
    let live = require_live(&state)?;

    if let Some(max_capital_usd) = body.max_capital_usd {
        if !max_capital_usd.is_finite() || max_capital_usd < 0.0 {
            return Err(ApiError::BadRequest(
                "max_capital_usd must be a non-negative finite number".to_string(),
            ));
        }
        live.set_max_capital_usd(max_capital_usd);
    }

    if let Some(min_confidence) = body.min_confidence {
        if !min_confidence.is_finite() || !(0.0..=1.0).contains(&min_confidence) {
            return Err(ApiError::BadRequest(
                "min_confidence must be a finite number between 0.0 and 1.0".to_string(),
            ));
        }
        live.set_min_confidence(min_confidence);
    }

    if let Some(take_profit_percent) = body.take_profit_percent {
        if !take_profit_percent.is_finite() || take_profit_percent <= 0.0 {
            return Err(ApiError::BadRequest(
                "take_profit_percent must be a positive finite number".to_string(),
            ));
        }
        live.set_take_profit_percent(take_profit_percent);
    }

    Ok(Json(live.status()))
}
