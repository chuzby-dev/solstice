//! Hard risk limits: position, daily loss, exposure, concentration, and
//! order limits. Each is a pure check — no I/O, no shared state — so they
//! compose cleanly inside [`crate::risk::pretrade::PreTradeRiskChecker`].

use crate::error::{ExecutionError, ExecutionResult};

#[derive(Debug, Clone, Copy)]
pub struct PositionLimits {
    pub max_single_position_usd: u64,
    pub max_position_percent: f64,
    pub min_position_size_usd: u64,
    pub max_open_positions: usize,
}

impl PositionLimits {
    pub fn can_open(
        &self,
        portfolio_value_usd: u64,
        new_position_usd: u64,
        current_positions: usize,
    ) -> ExecutionResult<()> {
        if new_position_usd > self.max_single_position_usd {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "position ${new_position_usd} exceeds max single position ${}",
                self.max_single_position_usd
            )));
        }

        if new_position_usd < self.min_position_size_usd {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "position ${new_position_usd} below min viable size ${}",
                self.min_position_size_usd
            )));
        }

        if portfolio_value_usd > 0 {
            let pct = new_position_usd as f64 / portfolio_value_usd as f64;
            if pct > self.max_position_percent {
                return Err(ExecutionError::RiskLimitViolated(format!(
                    "position is {:.1}% of portfolio, exceeds max {:.1}%",
                    pct * 100.0,
                    self.max_position_percent * 100.0
                )));
            }
        }

        if current_positions >= self.max_open_positions {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "already at max open positions ({})",
                self.max_open_positions
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DailyLossLimits {
    pub max_daily_loss_usd: u64,
    pub max_daily_loss_percent: f64,
}

impl DailyLossLimits {
    /// `daily_loss_usd` is expected non-positive (a loss); its magnitude
    /// is compared against the limits.
    pub fn check_loss(&self, daily_loss_usd: i64, portfolio_value_usd: u64) -> ExecutionResult<()> {
        let loss = daily_loss_usd.unsigned_abs();

        if loss > self.max_daily_loss_usd {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "daily loss ${loss} exceeds max ${}",
                self.max_daily_loss_usd
            )));
        }

        if portfolio_value_usd > 0 {
            let pct = loss as f64 / portfolio_value_usd as f64;
            if pct > self.max_daily_loss_percent {
                return Err(ExecutionError::RiskLimitViolated(format!(
                    "daily loss is {:.1}% of portfolio, exceeds max {:.1}%",
                    pct * 100.0,
                    self.max_daily_loss_percent * 100.0
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExposureLimits {
    pub max_total_exposure_usd: u64,
    pub max_leverage: f64,
}

impl ExposureLimits {
    pub fn can_increase_exposure(
        &self,
        current_exposure_usd: u64,
        additional_exposure_usd: u64,
        portfolio_value_usd: u64,
    ) -> ExecutionResult<()> {
        let total = current_exposure_usd.saturating_add(additional_exposure_usd);

        if total > self.max_total_exposure_usd {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "total exposure ${total} would exceed max ${}",
                self.max_total_exposure_usd
            )));
        }

        if portfolio_value_usd > 0 {
            let leverage = total as f64 / portfolio_value_usd as f64;
            if leverage > self.max_leverage {
                return Err(ExecutionError::RiskLimitViolated(format!(
                    "leverage {leverage:.2}x would exceed max {:.2}x",
                    self.max_leverage
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConcentrationLimits {
    pub max_single_asset_percent: f64,
}

impl ConcentrationLimits {
    pub fn check_concentration(
        &self,
        position_usd: u64,
        portfolio_value_usd: u64,
    ) -> ExecutionResult<()> {
        if portfolio_value_usd == 0 {
            return Ok(());
        }
        let pct = position_usd as f64 / portfolio_value_usd as f64;
        if pct > self.max_single_asset_percent {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "concentration {:.1}% exceeds max {:.1}%",
                pct * 100.0,
                self.max_single_asset_percent * 100.0
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OrderLimits {
    pub max_order_size_usd: u64,
    /// Maximum acceptable slippage, as a decimal fraction (e.g. `0.02` = 2%).
    pub max_slippage_percent: f64,
}

impl OrderLimits {
    pub fn can_submit_order(
        &self,
        order_size_usd: u64,
        simulated_slippage: f64,
    ) -> ExecutionResult<()> {
        if order_size_usd > self.max_order_size_usd {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "order ${order_size_usd} exceeds max ${}",
                self.max_order_size_usd
            )));
        }

        if simulated_slippage > self.max_slippage_percent {
            return Err(ExecutionError::RiskLimitViolated(format!(
                "slippage {:.2}% exceeds max {:.2}%",
                simulated_slippage * 100.0,
                self.max_slippage_percent * 100.0
            )));
        }

        Ok(())
    }
}

/// All hard risk limits, bundled for convenient configuration/threading.
#[derive(Debug, Clone, Copy)]
pub struct RiskLimits {
    pub position: PositionLimits,
    pub daily_loss: DailyLossLimits,
    pub exposure: ExposureLimits,
    pub concentration: ConcentrationLimits,
    pub order: OrderLimits,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_limits_valid() {
        let limits = PositionLimits {
            max_single_position_usd: 100_000,
            max_position_percent: 0.25,
            min_position_size_usd: 1_000,
            max_open_positions: 50,
        };
        assert!(limits.can_open(1_000_000, 50_000, 10).is_ok());
    }

    #[test]
    fn test_position_limits_exceeds_single_position() {
        let limits = PositionLimits {
            max_single_position_usd: 100_000,
            max_position_percent: 0.25,
            min_position_size_usd: 1_000,
            max_open_positions: 50,
        };
        assert!(limits.can_open(1_000_000, 150_000, 10).is_err());
    }

    #[test]
    fn test_position_limits_exceeds_portfolio_percent() {
        let limits = PositionLimits {
            max_single_position_usd: 1_000_000,
            max_position_percent: 0.25,
            min_position_size_usd: 1_000,
            max_open_positions: 50,
        };
        assert!(limits.can_open(1_000_000, 300_000, 10).is_err());
    }

    #[test]
    fn test_position_limits_below_minimum() {
        let limits = PositionLimits {
            max_single_position_usd: 100_000,
            max_position_percent: 0.25,
            min_position_size_usd: 1_000,
            max_open_positions: 50,
        };
        assert!(limits.can_open(1_000_000, 500, 10).is_err());
    }

    #[test]
    fn test_position_limits_max_open_reached() {
        let limits = PositionLimits {
            max_single_position_usd: 100_000,
            max_position_percent: 0.25,
            min_position_size_usd: 1_000,
            max_open_positions: 5,
        };
        assert!(limits.can_open(1_000_000, 10_000, 5).is_err());
    }

    #[test]
    fn test_daily_loss_within_limit() {
        let limits = DailyLossLimits {
            max_daily_loss_usd: 50_000,
            max_daily_loss_percent: 0.05,
        };
        assert!(limits.check_loss(-10_000, 1_000_000).is_ok());
    }

    #[test]
    fn test_daily_loss_exceeds_absolute_limit() {
        let limits = DailyLossLimits {
            max_daily_loss_usd: 50_000,
            max_daily_loss_percent: 0.50,
        };
        assert!(limits.check_loss(-60_000, 1_000_000).is_err());
    }

    #[test]
    fn test_daily_loss_exceeds_percent_limit() {
        let limits = DailyLossLimits {
            max_daily_loss_usd: 1_000_000,
            max_daily_loss_percent: 0.05,
        };
        assert!(limits.check_loss(-60_000, 1_000_000).is_err());
    }

    #[test]
    fn test_exposure_within_limit() {
        let limits = ExposureLimits {
            max_total_exposure_usd: 500_000,
            max_leverage: 1.0,
        };
        assert!(limits
            .can_increase_exposure(100_000, 50_000, 1_000_000)
            .is_ok());
    }

    #[test]
    fn test_exposure_exceeds_leverage() {
        let limits = ExposureLimits {
            max_total_exposure_usd: 5_000_000,
            max_leverage: 1.0,
        };
        assert!(limits
            .can_increase_exposure(900_000, 200_000, 1_000_000)
            .is_err());
    }

    #[test]
    fn test_concentration_within_limit() {
        let limits = ConcentrationLimits {
            max_single_asset_percent: 0.3,
        };
        assert!(limits.check_concentration(200_000, 1_000_000).is_ok());
    }

    #[test]
    fn test_concentration_exceeds_limit() {
        let limits = ConcentrationLimits {
            max_single_asset_percent: 0.3,
        };
        assert!(limits.check_concentration(400_000, 1_000_000).is_err());
    }

    #[test]
    fn test_order_limits_valid() {
        let limits = OrderLimits {
            max_order_size_usd: 50_000,
            max_slippage_percent: 0.02,
        };
        assert!(limits.can_submit_order(10_000, 0.005).is_ok());
    }

    #[test]
    fn test_order_limits_exceeds_slippage() {
        let limits = OrderLimits {
            max_order_size_usd: 50_000,
            max_slippage_percent: 0.02,
        };
        assert!(limits.can_submit_order(10_000, 0.05).is_err());
    }
}
