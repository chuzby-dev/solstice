//! Real-time risk monitoring: tracks portfolio risk metrics over time and
//! trips a circuit breaker when the daily loss limit is breached.

use crate::risk::limits::RiskLimits;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tracing::{error, warn};

#[derive(Debug, Clone, PartialEq)]
pub enum RiskLimitStatus {
    Healthy,
    Warning { limit: String, usage: f64 },
    Critical { limit: String, usage: f64 },
    Breached { limit: String },
}

#[derive(Debug, Clone)]
pub struct PortfolioRiskMetrics {
    pub timestamp: DateTime<Utc>,
    pub total_positions: usize,
    pub total_exposure_usd: u64,
    pub daily_pnl_usd: i64,
    pub limits_status: RiskLimitStatus,
}

/// Tracks portfolio risk over time against [`RiskLimits`], and trips a
/// circuit breaker (manual-reset only, per `docs/RISK_MANAGEMENT.md`'s
/// fail-safe philosophy) when the daily loss limit is breached.
pub struct RiskMonitor {
    limits: RiskLimits,
    warning_threshold: f64,
    critical_threshold: f64,
    metrics_history: Mutex<VecDeque<PortfolioRiskMetrics>>,
    max_history: usize,
    circuit_breaker_tripped: AtomicBool,
}

impl RiskMonitor {
    pub fn new(limits: RiskLimits) -> Self {
        RiskMonitor {
            limits,
            warning_threshold: 0.8,
            critical_threshold: 0.95,
            metrics_history: Mutex::new(VecDeque::new()),
            max_history: 10_000,
            circuit_breaker_tripped: AtomicBool::new(false),
        }
    }

    pub fn with_thresholds(mut self, warning: f64, critical: f64) -> Self {
        self.warning_threshold = warning;
        self.critical_threshold = critical;
        self
    }

    /// Record a new snapshot of portfolio risk, evaluate it against the
    /// daily loss limit, and trip the circuit breaker if breached.
    pub fn update(
        &self,
        total_positions: usize,
        total_exposure_usd: u64,
        daily_pnl_usd: i64,
        portfolio_value_usd: u64,
    ) -> PortfolioRiskMetrics {
        let limits_status = self.check_daily_loss(daily_pnl_usd, portfolio_value_usd);

        match &limits_status {
            RiskLimitStatus::Warning { .. } | RiskLimitStatus::Critical { .. } => {
                warn!("Risk limit approaching: {:?}", limits_status);
            }
            RiskLimitStatus::Breached { limit } => {
                error!("RISK LIMIT BREACHED: {}", limit);
                self.circuit_breaker_tripped.store(true, Ordering::SeqCst);
            }
            RiskLimitStatus::Healthy => {}
        }

        let metrics = PortfolioRiskMetrics {
            timestamp: Utc::now(),
            total_positions,
            total_exposure_usd,
            daily_pnl_usd,
            limits_status,
        };

        if let Ok(mut history) = self.metrics_history.lock() {
            history.push_back(metrics.clone());
            while history.len() > self.max_history {
                history.pop_front();
            }
        }

        metrics
    }

    fn check_daily_loss(&self, daily_pnl_usd: i64, portfolio_value_usd: u64) -> RiskLimitStatus {
        if daily_pnl_usd >= 0 {
            return RiskLimitStatus::Healthy;
        }
        let loss = daily_pnl_usd.unsigned_abs();

        if loss > self.limits.daily_loss.max_daily_loss_usd {
            return RiskLimitStatus::Breached {
                limit: "daily_loss".to_string(),
            };
        }

        let usage = loss as f64 / self.limits.daily_loss.max_daily_loss_usd as f64;
        if portfolio_value_usd > 0 {
            let pct = loss as f64 / portfolio_value_usd as f64;
            if pct > self.limits.daily_loss.max_daily_loss_percent {
                return RiskLimitStatus::Breached {
                    limit: "daily_loss_percent".to_string(),
                };
            }
        }

        if usage >= self.critical_threshold {
            RiskLimitStatus::Critical {
                limit: "daily_loss".to_string(),
                usage,
            }
        } else if usage >= self.warning_threshold {
            RiskLimitStatus::Warning {
                limit: "daily_loss".to_string(),
                usage,
            }
        } else {
            RiskLimitStatus::Healthy
        }
    }

    pub fn is_circuit_breaker_tripped(&self) -> bool {
        self.circuit_breaker_tripped.load(Ordering::SeqCst)
    }

    /// Manually reset the circuit breaker. Per the fail-safe philosophy,
    /// nothing in this crate resets it automatically.
    pub fn reset_circuit_breaker(&self) {
        self.circuit_breaker_tripped.store(false, Ordering::SeqCst);
    }

    pub fn history(&self) -> Vec<PortfolioRiskMetrics> {
        self.metrics_history
            .lock()
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::risk::limits::{
        ConcentrationLimits, DailyLossLimits, ExposureLimits, OrderLimits, PositionLimits,
    };

    fn test_limits() -> RiskLimits {
        RiskLimits {
            position: PositionLimits {
                max_single_position_usd: 100_000,
                max_position_percent: 0.25,
                min_position_size_usd: 1_000,
                max_open_positions: 50,
            },
            daily_loss: DailyLossLimits {
                max_daily_loss_usd: 50_000,
                max_daily_loss_percent: 0.5,
            },
            exposure: ExposureLimits {
                max_total_exposure_usd: 500_000,
                max_leverage: 1.0,
            },
            concentration: ConcentrationLimits {
                max_single_asset_percent: 0.3,
            },
            order: OrderLimits {
                max_order_size_usd: 50_000,
                max_slippage_percent: 0.02,
            },
        }
    }

    #[test]
    fn test_healthy_when_no_loss() {
        let monitor = RiskMonitor::new(test_limits());
        let metrics = monitor.update(5, 100_000, 1_000, 1_000_000);
        assert_eq!(metrics.limits_status, RiskLimitStatus::Healthy);
        assert!(!monitor.is_circuit_breaker_tripped());
    }

    #[test]
    fn test_warning_at_80_percent_usage() {
        let monitor = RiskMonitor::new(test_limits());
        // 80% of 50,000 = 40,000 loss.
        let metrics = monitor.update(5, 100_000, -40_000, 1_000_000);
        assert!(matches!(
            metrics.limits_status,
            RiskLimitStatus::Warning { .. }
        ));
        assert!(!monitor.is_circuit_breaker_tripped());
    }

    #[test]
    fn test_breach_trips_circuit_breaker() {
        let monitor = RiskMonitor::new(test_limits());
        let metrics = monitor.update(5, 100_000, -60_000, 1_000_000);
        assert!(matches!(
            metrics.limits_status,
            RiskLimitStatus::Breached { .. }
        ));
        assert!(monitor.is_circuit_breaker_tripped());
    }

    #[test]
    fn test_manual_reset_required() {
        let monitor = RiskMonitor::new(test_limits());
        monitor.update(5, 100_000, -60_000, 1_000_000);
        assert!(monitor.is_circuit_breaker_tripped());

        // A subsequent healthy update does NOT clear it automatically.
        monitor.update(5, 100_000, 1_000, 1_000_000);
        assert!(monitor.is_circuit_breaker_tripped());

        monitor.reset_circuit_breaker();
        assert!(!monitor.is_circuit_breaker_tripped());
    }

    #[test]
    fn test_history_recorded() {
        let monitor = RiskMonitor::new(test_limits());
        monitor.update(5, 100_000, 1_000, 1_000_000);
        monitor.update(6, 110_000, 2_000, 1_010_000);

        assert_eq!(monitor.history().len(), 2);
    }
}
