//! API response DTOs.
//!
//! Deliberately not the same types the engine/execution crates use
//! internally: an API response shape is a contract with clients and
//! shouldn't change just because an internal refactor changes a domain
//! type's fields.

use chrono::{DateTime, Utc};
use serde::Serialize;
use solstice_execution::order_manager::{Order, OrderStatus};
use solstice_execution::TradeApproval;
use solstice_simulation::PortfolioSnapshot;

#[derive(Debug, Clone, Serialize)]
pub struct StatusResponse {
    pub status: &'static str,
    pub monitored_pairs: Vec<String>,
    pub open_positions: usize,
    pub total_value_usd: f64,
    pub circuit_breaker_tripped: bool,
}

/// A wallet's public address and its current SOL balance. Read-only by
/// design -- there is no field or endpoint here that can move funds; this
/// server only reports what it can see.
#[derive(Debug, Clone, Serialize)]
pub struct WalletResponse {
    pub address: String,
    pub balance_lamports: u64,
    pub balance_sol: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PositionsResponse {
    pub positions: Vec<solstice_simulation::PositionSnapshot>,
}

impl From<PortfolioSnapshot> for PositionsResponse {
    fn from(snapshot: PortfolioSnapshot) -> Self {
        PositionsResponse {
            positions: snapshot.positions,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PerformanceResponse {
    pub cash_usd: f64,
    pub realized_pnl_usd: f64,
    pub unrealized_pnl_usd: f64,
    pub total_value_usd: f64,
}

impl From<PortfolioSnapshot> for PerformanceResponse {
    fn from(snapshot: PortfolioSnapshot) -> Self {
        PerformanceResponse {
            cash_usd: snapshot.cash_usd,
            realized_pnl_usd: snapshot.realized_pnl_usd,
            unrealized_pnl_usd: snapshot.unrealized_pnl_usd,
            total_value_usd: snapshot.total_value_usd,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatusDto {
    Submitted,
    PartiallyFilled,
    Filled,
    Failed,
    Cancelled,
}

impl From<&OrderStatus> for OrderStatusDto {
    fn from(status: &OrderStatus) -> Self {
        match status {
            OrderStatus::Submitted => OrderStatusDto::Submitted,
            OrderStatus::PartiallyFilled => OrderStatusDto::PartiallyFilled,
            OrderStatus::Filled => OrderStatusDto::Filled,
            OrderStatus::Failed => OrderStatusDto::Failed,
            OrderStatus::Cancelled => OrderStatusDto::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TradeResponse {
    pub order_id: String,
    pub strategy: String,
    pub status: OrderStatusDto,
    pub size_usd: u64,
    pub filled_amount: u64,
    pub base_mint: String,
    pub quote_mint: String,
    pub approved: bool,
    pub rejection_reason: Option<String>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&Order> for TradeResponse {
    fn from(order: &Order) -> Self {
        let (approved, rejection_reason) = match &order.plan.approval {
            TradeApproval::Approved => (true, None),
            TradeApproval::Rejected { reason } => (false, Some(reason.clone())),
        };

        TradeResponse {
            order_id: order.id.clone(),
            strategy: order.plan.signal.strategy.clone(),
            status: (&order.status).into(),
            size_usd: order.plan.size_usd,
            filled_amount: order.filled_amount,
            base_mint: order.plan.pair.base.to_string(),
            quote_mint: order.plan.pair.quote.to_string(),
            approved,
            rejection_reason,
            failure_reason: order.failure_reason.clone(),
            created_at: order.created_at,
            updated_at: order.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TradesResponse {
    pub trades: Vec<TradeResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_status_dto_serializes_snake_case() {
        let dto = OrderStatusDto::PartiallyFilled;
        let json = serde_json::to_string(&dto).unwrap();
        assert_eq!(json, "\"partially_filled\"");
    }
}
