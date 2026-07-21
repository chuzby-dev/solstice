//! Order lifecycle tracking: submission, fills, and terminal states.

use crate::error::{ExecutionError, ExecutionResult};
use crate::planner::ExecutionPlan;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderStatus {
    Submitted,
    PartiallyFilled,
    Filled,
    Failed,
    Cancelled,
}

impl OrderStatus {
    /// Whether this status is terminal — no further transitions allowed.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OrderStatus::Filled | OrderStatus::Failed | OrderStatus::Cancelled
        )
    }
}

/// A single fill against an order.
#[derive(Debug, Clone)]
pub struct Fill {
    pub amount: u64,
    pub price: f64,
    pub fee: f64,
    pub timestamp: DateTime<Utc>,
    pub tx_signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub id: String,
    pub plan: ExecutionPlan,
    pub status: OrderStatus,
    pub fills: Vec<Fill>,
    pub filled_amount: u64,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Order {
    pub fn remaining_amount(&self) -> u64 {
        self.plan.size_usd.saturating_sub(self.filled_amount)
    }
}

/// Tracks submitted orders and their fills in memory.
#[derive(Default)]
pub struct OrderManager {
    orders: RwLock<HashMap<String, Order>>,
}

impl OrderManager {
    pub fn new() -> Self {
        OrderManager {
            orders: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new order for a plan that passed pre-trade risk checks.
    /// Rejects plans whose `approval` wasn't `Approved` — an order should
    /// never exist for a trade the risk checker didn't clear.
    pub fn submit(&self, plan: ExecutionPlan) -> ExecutionResult<String> {
        if !plan.approval.is_approved() {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "cannot submit order for a rejected plan: {:?}",
                plan.approval
            )));
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let order = Order {
            id: id.clone(),
            plan,
            status: OrderStatus::Submitted,
            fills: Vec::new(),
            filled_amount: 0,
            failure_reason: None,
            created_at: now,
            updated_at: now,
        };

        let mut orders = self
            .orders
            .write()
            .map_err(|_| ExecutionError::OrderNotFound("lock poisoned".to_string()))?;
        orders.insert(id.clone(), order);
        Ok(id)
    }

    /// Record a fill against an order, transitioning it to
    /// `PartiallyFilled` or `Filled` depending on cumulative fill amount.
    pub fn record_fill(&self, order_id: &str, fill: Fill) -> ExecutionResult<()> {
        let mut orders = self
            .orders
            .write()
            .map_err(|_| ExecutionError::OrderNotFound("lock poisoned".to_string()))?;
        let order = orders
            .get_mut(order_id)
            .ok_or_else(|| ExecutionError::OrderNotFound(order_id.to_string()))?;

        if order.status.is_terminal() {
            return Err(ExecutionError::InvalidOrderTransition(format!(
                "order {order_id} is already {:?}",
                order.status
            )));
        }

        order.filled_amount = order.filled_amount.saturating_add(fill.amount);
        order.fills.push(fill);
        order.status = if order.filled_amount >= order.plan.size_usd {
            OrderStatus::Filled
        } else {
            OrderStatus::PartiallyFilled
        };
        order.updated_at = Utc::now();
        Ok(())
    }

    pub fn fail(&self, order_id: &str, reason: String) -> ExecutionResult<()> {
        self.transition_to_terminal(order_id, OrderStatus::Failed, Some(reason))
    }

    pub fn cancel(&self, order_id: &str) -> ExecutionResult<()> {
        self.transition_to_terminal(order_id, OrderStatus::Cancelled, None)
    }

    fn transition_to_terminal(
        &self,
        order_id: &str,
        status: OrderStatus,
        reason: Option<String>,
    ) -> ExecutionResult<()> {
        let mut orders = self
            .orders
            .write()
            .map_err(|_| ExecutionError::OrderNotFound("lock poisoned".to_string()))?;
        let order = orders
            .get_mut(order_id)
            .ok_or_else(|| ExecutionError::OrderNotFound(order_id.to_string()))?;

        if order.status.is_terminal() {
            return Err(ExecutionError::InvalidOrderTransition(format!(
                "order {order_id} is already {:?}",
                order.status
            )));
        }

        order.status = status;
        order.failure_reason = reason;
        order.updated_at = Utc::now();
        Ok(())
    }

    pub fn get(&self, order_id: &str) -> ExecutionResult<Order> {
        self.orders
            .read()
            .map_err(|_| ExecutionError::OrderNotFound("lock poisoned".to_string()))?
            .get(order_id)
            .cloned()
            .ok_or_else(|| ExecutionError::OrderNotFound(order_id.to_string()))
    }

    /// Orders not yet in a terminal state.
    pub fn open_orders(&self) -> Vec<Order> {
        self.orders
            .read()
            .map(|orders| {
                orders
                    .values()
                    .filter(|o| !o.status.is_terminal())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every order regardless of status, newest first.
    pub fn all_orders(&self) -> Vec<Order> {
        let mut orders: Vec<Order> = self
            .orders
            .read()
            .map(|orders| orders.values().cloned().collect())
            .unwrap_or_default();
        orders.sort_by_key(|o| std::cmp::Reverse(o.created_at));
        orders
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::TradeApproval;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::{Signal, SignalType, TokenPair};
    use solstice_dex::{Quote, RouteSegment};

    fn approved_plan() -> ExecutionPlan {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let signal = Signal::new("test".to_string(), SignalType::Buy { pair }, 0.9);
        ExecutionPlan {
            signal,
            pair,
            quote: Quote {
                in_amount: 1_000,
                out_amount: 2_000,
                fee_amount: 5,
                fee_bps: 25,
                price_impact: 0.001,
                liquidity: 10_000,
                route: vec![RouteSegment {
                    dex: "Test".to_string(),
                    input_mint: pair.quote,
                    output_mint: pair.base,
                    input_amount: 1_000,
                    output_amount: 2_000,
                }],
                timestamp: Utc::now(),
            },
            size_usd: 1_000,
            approval: TradeApproval::Approved,
        }
    }

    fn rejected_plan() -> ExecutionPlan {
        let mut plan = approved_plan();
        plan.approval = TradeApproval::Rejected {
            reason: "too large".to_string(),
        };
        plan
    }

    #[test]
    fn test_submit_rejected_plan_fails() {
        let manager = OrderManager::new();
        assert!(manager.submit(rejected_plan()).is_err());
    }

    #[test]
    fn test_submit_and_get() {
        let manager = OrderManager::new();
        let id = manager.submit(approved_plan()).unwrap();
        let order = manager.get(&id).unwrap();
        assert_eq!(order.status, OrderStatus::Submitted);
    }

    #[test]
    fn test_partial_fill_then_full_fill() {
        let manager = OrderManager::new();
        let id = manager.submit(approved_plan()).unwrap();

        manager
            .record_fill(
                &id,
                Fill {
                    amount: 400,
                    price: 2.0,
                    fee: 1.0,
                    timestamp: Utc::now(),
                    tx_signature: None,
                },
            )
            .unwrap();
        assert_eq!(
            manager.get(&id).unwrap().status,
            OrderStatus::PartiallyFilled
        );

        manager
            .record_fill(
                &id,
                Fill {
                    amount: 600,
                    price: 2.0,
                    fee: 1.0,
                    timestamp: Utc::now(),
                    tx_signature: None,
                },
            )
            .unwrap();
        let order = manager.get(&id).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.filled_amount, 1_000);
    }

    #[test]
    fn test_cannot_fill_terminal_order() {
        let manager = OrderManager::new();
        let id = manager.submit(approved_plan()).unwrap();
        manager.cancel(&id).unwrap();

        let result = manager.record_fill(
            &id,
            Fill {
                amount: 100,
                price: 2.0,
                fee: 0.0,
                timestamp: Utc::now(),
                tx_signature: None,
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_open_orders_excludes_terminal() {
        let manager = OrderManager::new();
        let open_id = manager.submit(approved_plan()).unwrap();
        let closed_id = manager.submit(approved_plan()).unwrap();
        manager.cancel(&closed_id).unwrap();

        let open = manager.open_orders();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, open_id);
    }

    #[test]
    fn test_fail_order() {
        let manager = OrderManager::new();
        let id = manager.submit(approved_plan()).unwrap();
        manager.fail(&id, "simulation failed".to_string()).unwrap();

        let order = manager.get(&id).unwrap();
        assert_eq!(order.status, OrderStatus::Failed);
        assert_eq!(order.failure_reason, Some("simulation failed".to_string()));
    }

    #[test]
    fn test_all_orders_includes_terminal() {
        let manager = OrderManager::new();
        let open_id = manager.submit(approved_plan()).unwrap();
        let closed_id = manager.submit(approved_plan()).unwrap();
        manager.cancel(&closed_id).unwrap();

        let all = manager.all_orders();
        let ids: Vec<_> = all.iter().map(|o| o.id.clone()).collect();
        assert_eq!(all.len(), 2);
        assert!(ids.contains(&open_id));
        assert!(ids.contains(&closed_id));
    }
}
