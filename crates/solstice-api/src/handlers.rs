//! REST endpoint handlers.

use crate::dto::{
    PerformanceResponse, PositionsResponse, StatusResponse, TradeResponse, TradesResponse,
    WalletResponse,
};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;

const LAMPORTS_PER_SOL: f64 = 1_000_000_000.0;

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

/// Read-only wallet status: address and current balance. `404` if no
/// wallet is configured for this server (`WALLET_KEYPAIR_PATH` unset).
/// There is deliberately no corresponding write/send endpoint here.
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

    Ok(Json(WalletResponse {
        address: wallet.pubkey.to_string(),
        balance_lamports,
        balance_sol: balance_lamports as f64 / LAMPORTS_PER_SOL,
    }))
}
