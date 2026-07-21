//! Backtest performance calculation and report generation.

use chrono::{DateTime, Utc};
use serde::Serialize;
use solstice_core::types::TokenPair;

/// Portfolio value at one point along the replay.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct EquityPoint {
    pub timestamp: DateTime<Utc>,
    pub total_value_usd: f64,
}

/// One simulated fill against a signal.
#[derive(Debug, Clone, Serialize)]
pub struct TradeRecord {
    pub strategy: String,
    pub pair: String,
    pub is_buy: bool,
    pub size_usd: u64,
    pub price: f64,
    pub fee_usd: f64,
    pub timestamp: DateTime<Utc>,
}

/// A fully closed round-trip position (currently only produced by stop-loss
/// exits — no strategy shipped in this workspace emits a `Sell`/`Close`
/// signal yet, matching the live paper-trading engine's same limitation).
#[derive(Debug, Clone, Serialize)]
pub struct ClosedPositionRecord {
    pub pair: String,
    pub quantity: i64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub realized_pnl_usd: f64,
    pub opened_at: DateTime<Utc>,
    pub closed_at: DateTime<Utc>,
    pub reason: String,
}

/// Summary statistics computed from an equity curve and trade log.
#[derive(Debug, Clone, Serialize)]
pub struct PerformanceMetrics {
    pub initial_capital_usd: f64,
    pub final_equity_usd: f64,
    pub total_return_pct: f64,
    pub max_drawdown_pct: f64,
    /// Sharpe ratio computed from per-tick equity returns (mean / stddev).
    /// **Not annualized** — replay tick spacing is whatever the input data
    /// uses, not a fixed period, so scaling by `sqrt(periods_per_year)` is
    /// left to the caller, who knows their data's actual frequency.
    pub sharpe_ratio: Option<f64>,
    pub num_fills: usize,
    pub total_fees_usd: f64,
    pub num_closed_positions: usize,
    /// Fraction of closed positions with positive realized P&L. `None` if
    /// no positions closed during the replay.
    pub win_rate: Option<f64>,
}

impl PerformanceMetrics {
    pub fn compute(
        initial_capital_usd: f64,
        equity_curve: &[EquityPoint],
        fills: &[TradeRecord],
        closed_positions: &[ClosedPositionRecord],
    ) -> Self {
        let final_equity_usd = equity_curve
            .last()
            .map(|p| p.total_value_usd)
            .unwrap_or(initial_capital_usd);

        let total_return_pct = if initial_capital_usd > 0.0 {
            (final_equity_usd - initial_capital_usd) / initial_capital_usd * 100.0
        } else {
            0.0
        };

        let max_drawdown_pct = max_drawdown(equity_curve);
        let sharpe_ratio = sharpe(equity_curve);

        let total_fees_usd = fills.iter().map(|f| f.fee_usd).sum();

        let win_rate = if closed_positions.is_empty() {
            None
        } else {
            let wins = closed_positions
                .iter()
                .filter(|p| p.realized_pnl_usd > 0.0)
                .count();
            Some(wins as f64 / closed_positions.len() as f64)
        };

        PerformanceMetrics {
            initial_capital_usd,
            final_equity_usd,
            total_return_pct,
            max_drawdown_pct,
            sharpe_ratio,
            num_fills: fills.len(),
            total_fees_usd,
            num_closed_positions: closed_positions.len(),
            win_rate,
        }
    }
}

fn max_drawdown(equity_curve: &[EquityPoint]) -> f64 {
    let mut peak = f64::MIN;
    let mut worst = 0.0_f64;
    for point in equity_curve {
        peak = peak.max(point.total_value_usd);
        if peak > 0.0 {
            let drawdown = (peak - point.total_value_usd) / peak * 100.0;
            worst = worst.max(drawdown);
        }
    }
    worst
}

fn sharpe(equity_curve: &[EquityPoint]) -> Option<f64> {
    if equity_curve.len() < 3 {
        return None;
    }
    let returns: Vec<f64> = equity_curve
        .windows(2)
        .filter_map(|w| {
            let (prev, curr) = (w[0].total_value_usd, w[1].total_value_usd);
            if prev > 0.0 {
                Some((curr - prev) / prev)
            } else {
                None
            }
        })
        .collect();

    if returns.len() < 2 {
        return None;
    }

    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance =
        returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
    let stddev = variance.sqrt();

    if stddev == 0.0 {
        None
    } else {
        Some(mean / stddev)
    }
}

/// The full output of a backtest run: everything needed to inspect,
/// chart, or further analyze what happened, plus the summary metrics.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestReport {
    pub pair: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub equity_curve: Vec<EquityPoint>,
    pub fills: Vec<TradeRecord>,
    pub closed_positions: Vec<ClosedPositionRecord>,
    pub metrics: PerformanceMetrics,
}

impl BacktestReport {
    pub fn to_json_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// A human-readable Markdown summary — the "report generation" this
    /// engine produces alongside the machine-readable JSON.
    pub fn to_markdown(&self) -> String {
        let m = &self.metrics;
        let mut out = String::new();
        out.push_str(&format!("# Backtest report — {}\n\n", self.pair));
        out.push_str(&format!(
            "Replayed **{}** to **{}** ({} ticks)\n\n",
            self.start,
            self.end,
            self.equity_curve.len()
        ));
        out.push_str("## Performance\n\n");
        out.push_str(&format!(
            "- Initial capital: ${:.2}\n",
            m.initial_capital_usd
        ));
        out.push_str(&format!("- Final equity: ${:.2}\n", m.final_equity_usd));
        out.push_str(&format!("- Total return: {:.2}%\n", m.total_return_pct));
        out.push_str(&format!("- Max drawdown: {:.2}%\n", m.max_drawdown_pct));
        match m.sharpe_ratio {
            Some(s) => out.push_str(&format!(
                "- Sharpe ratio (per-tick, not annualized): {s:.3}\n"
            )),
            None => out.push_str("- Sharpe ratio: n/a (insufficient data)\n"),
        }
        out.push_str(&format!(
            "- Fills: {} (${:.2} in fees)\n",
            m.num_fills, m.total_fees_usd
        ));
        out.push_str(&format!("- Closed positions: {}", m.num_closed_positions));
        match m.win_rate {
            Some(w) => out.push_str(&format!(" ({:.1}% win rate)\n", w * 100.0)),
            None => out.push('\n'),
        }
        out
    }
}

pub(crate) fn pair_label(pair: &TokenPair) -> String {
    pair.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(usd: f64) -> EquityPoint {
        EquityPoint {
            timestamp: Utc::now(),
            total_value_usd: usd,
        }
    }

    #[test]
    fn test_total_return_and_final_equity() {
        let curve = vec![point(10_000.0), point(11_000.0)];
        let metrics = PerformanceMetrics::compute(10_000.0, &curve, &[], &[]);
        assert_eq!(metrics.final_equity_usd, 11_000.0);
        assert!((metrics.total_return_pct - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_max_drawdown_tracks_peak_to_trough() {
        let curve = vec![
            point(10_000.0),
            point(12_000.0),
            point(9_000.0),
            point(11_000.0),
        ];
        let dd = max_drawdown(&curve);
        // From peak 12,000 down to 9,000 = 25% drawdown.
        assert!((dd - 25.0).abs() < 1e-9);
    }

    #[test]
    fn test_sharpe_none_with_too_few_points() {
        let curve = vec![point(10_000.0), point(10_100.0)];
        assert!(sharpe(&curve).is_none());
    }

    #[test]
    fn test_sharpe_none_with_zero_variance() {
        let curve = vec![point(10_000.0), point(10_000.0), point(10_000.0)];
        assert!(sharpe(&curve).is_none());
    }

    #[test]
    fn test_win_rate_computed_from_closed_positions() {
        let pair = "SOL/USDC".to_string();
        let closes = vec![
            ClosedPositionRecord {
                pair: pair.clone(),
                quantity: 10,
                entry_price: 100.0,
                exit_price: 110.0,
                realized_pnl_usd: 100.0,
                opened_at: Utc::now(),
                closed_at: Utc::now(),
                reason: "stop loss".to_string(),
            },
            ClosedPositionRecord {
                pair,
                quantity: 10,
                entry_price: 100.0,
                exit_price: 90.0,
                realized_pnl_usd: -100.0,
                opened_at: Utc::now(),
                closed_at: Utc::now(),
                reason: "stop loss".to_string(),
            },
        ];
        let metrics = PerformanceMetrics::compute(10_000.0, &[], &[], &closes);
        assert_eq!(metrics.win_rate, Some(0.5));
    }

    #[test]
    fn test_win_rate_none_when_no_closes() {
        let metrics = PerformanceMetrics::compute(10_000.0, &[], &[], &[]);
        assert_eq!(metrics.win_rate, None);
    }
}
