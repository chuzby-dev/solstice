//! REST endpoint handlers.

use crate::dto::{
    PerformanceResponse, PositionsResponse, StatusResponse, TradeResponse, TradesResponse,
};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;

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
