//! Automatic take-profit evaluation. Mirrors `StopLossManager` exactly,
//! just checking the opposite side of the same gain/loss percentage --
//! without this, a live position (opened by a strategy that itself never
//! signals an exit, e.g. SMA/SpreadArb here) only ever closes on a loss,
//! never on a gain, and would otherwise just sit indefinitely once
//! capital is fully deployed.

use solstice_core::types::{Position, PositionId};

/// A position the take-profit manager wants closed.
#[derive(Debug, Clone)]
pub struct TakeProfitTrigger {
    pub position_id: PositionId,
    pub gain_percent: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy)]
pub struct TakeProfitManager {
    /// Fractional gain (e.g. `0.05` = 5%) at which a position is flagged
    /// for exit.
    pub take_profit_percent: f64,
}

impl TakeProfitManager {
    pub fn new(take_profit_percent: f64) -> Self {
        TakeProfitManager {
            take_profit_percent,
        }
    }

    /// Evaluate every position and return triggers for any that have
    /// risen above the take-profit threshold. Only applies to long
    /// positions (`quantity > 0`), same rationale as `StopLossManager`.
    pub fn evaluate_targets(&self, positions: &[Position]) -> Vec<TakeProfitTrigger> {
        positions
            .iter()
            .filter(|p| p.quantity > 0 && p.entry_price > 0.0)
            .filter_map(|position| {
                let gain_pct =
                    (position.current_price - position.entry_price) / position.entry_price;
                if gain_pct > self.take_profit_percent {
                    Some(TakeProfitTrigger {
                        position_id: position.id,
                        gain_percent: gain_pct,
                        reason: format!("take profit triggered: {:.2}% gain", gain_pct * 100.0),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::TokenPair;

    fn position(entry: f64, current: f64, quantity: i64) -> Position {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let mut p = Position::new(pair, quantity, entry);
        p.current_price = current;
        p
    }

    #[test]
    fn test_no_trigger_within_tolerance() {
        let manager = TakeProfitManager::new(0.05);
        let positions = vec![position(100.0, 102.0, 100)]; // +2%
        assert!(manager.evaluate_targets(&positions).is_empty());
    }

    #[test]
    fn test_trigger_on_gain_beyond_threshold() {
        let manager = TakeProfitManager::new(0.05);
        let positions = vec![position(100.0, 110.0, 100)]; // +10%
        let triggers = manager.evaluate_targets(&positions);
        assert_eq!(triggers.len(), 1);
        assert!(triggers[0].gain_percent > 0.05);
    }

    #[test]
    fn test_no_trigger_on_loss() {
        let manager = TakeProfitManager::new(0.05);
        let positions = vec![position(100.0, 80.0, 100)];
        assert!(manager.evaluate_targets(&positions).is_empty());
    }

    #[test]
    fn test_short_positions_ignored() {
        let manager = TakeProfitManager::new(0.05);
        let positions = vec![position(100.0, 50.0, -100)];
        assert!(manager.evaluate_targets(&positions).is_empty());
    }
}
