//! REST endpoint handlers.

use crate::dto::{
    LiveConfigRequest, PerformanceResponse, PositionsResponse, StatusResponse, TradeResponse,
    TradesResponse, WalletResponse,
};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use solstice_execution::LiveStatusSnapshot;

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

    Ok(Json(live.status()))
}
