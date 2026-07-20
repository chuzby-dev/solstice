//! Automatic stop-loss evaluation.

use solstice_core::types::{Position, PositionId};

/// A position the stop-loss manager wants closed.
#[derive(Debug, Clone)]
pub struct StopLossTrigger {
    pub position_id: PositionId,
    pub loss_percent: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy)]
pub struct StopLossManager {
    /// Fractional loss (e.g. `0.05` = 5%) at which a position is flagged
    /// for exit.
    pub stop_loss_percent: f64,
}

impl StopLossManager {
    pub fn new(stop_loss_percent: f64) -> Self {
        StopLossManager { stop_loss_percent }
    }

    /// Evaluate every position and return triggers for any that have
    /// fallen below the stop-loss threshold. Only applies to long
    /// positions (`quantity > 0`) — short-position stop logic is inverted
    /// and isn't handled here since nothing in this workspace opens
    /// shorts yet.
    pub fn evaluate_stops(&self, positions: &[Position]) -> Vec<StopLossTrigger> {
        positions
            .iter()
            .filter(|p| p.quantity > 0 && p.entry_price > 0.0)
            .filter_map(|position| {
                let loss_pct =
                    (position.current_price - position.entry_price) / position.entry_price;
                if loss_pct < -self.stop_loss_percent {
                    Some(StopLossTrigger {
                        position_id: position.id,
                        loss_percent: loss_pct,
                        reason: format!("stop loss triggered: {:.2}% loss", loss_pct * 100.0),
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
        let manager = StopLossManager::new(0.05);
        let positions = vec![position(100.0, 98.0, 100)]; // -2%
        assert!(manager.evaluate_stops(&positions).is_empty());
    }

    #[test]
    fn test_trigger_on_loss_beyond_threshold() {
        let manager = StopLossManager::new(0.05);
        let positions = vec![position(100.0, 90.0, 100)]; // -10%
        let triggers = manager.evaluate_stops(&positions);
        assert_eq!(triggers.len(), 1);
        assert!(triggers[0].loss_percent < -0.05);
    }

    #[test]
    fn test_no_trigger_on_gain() {
        let manager = StopLossManager::new(0.05);
        let positions = vec![position(100.0, 120.0, 100)];
        assert!(manager.evaluate_stops(&positions).is_empty());
    }

    #[test]
    fn test_short_positions_ignored() {
        let manager = StopLossManager::new(0.05);
        let positions = vec![position(100.0, 200.0, -100)];
        assert!(manager.evaluate_stops(&positions).is_empty());
    }
}
