//! Position sizing: turning a signal's confidence into a concrete trade
//! size, via fractional Kelly criterion bounded by hard risk limits.

use crate::error::{ExecutionError, ExecutionResult};
use solstice_core::types::Signal;

/// Parameters controlling how large a position sizing decision may be.
#[derive(Debug, Clone, Copy)]
pub struct RiskParams {
    pub portfolio_value_usd: f64,
    pub available_capital_usd: f64,
    pub max_position_usd: f64,
    /// Maximum position size as a fraction of portfolio value (e.g. `0.25`).
    pub max_position_percent: f64,
    /// Scales the full Kelly fraction down for safety (e.g. `0.5` for
    /// half-Kelly). Full Kelly (`1.0`) is aggressive and rarely
    /// appropriate; values above `1.0` are not rejected but are unusual.
    pub kelly_fraction: f64,
    /// Assumed win/loss payoff ratio when a signal doesn't specify one.
    /// Used as the Kelly formula's `b` (average win size ÷ average loss
    /// size).
    pub default_win_loss_ratio: f64,
}

/// Computes trade sizes from signal confidence and risk parameters.
pub struct PositionSizer;

impl PositionSizer {
    /// Kelly criterion optimal bet fraction: `f* = p - (1-p)/b`, clamped
    /// to `[0.0, 1.0]` (never suggests betting a negative fraction, and
    /// never more than the entire bankroll even if the raw formula would).
    ///
    /// `win_probability` (`p`): probability the trade is profitable.
    /// `win_loss_ratio` (`b`): average win size ÷ average loss size.
    pub fn kelly_criterion(win_probability: f64, win_loss_ratio: f64) -> f64 {
        if win_loss_ratio <= 0.0 {
            return 0.0;
        }
        let p = win_probability.clamp(0.0, 1.0);
        let f = p - (1.0 - p) / win_loss_ratio;
        f.clamp(0.0, 1.0)
    }

    /// Calculate a position size in USD for `signal`, using its
    /// `confidence` as the Kelly win probability, scaled by
    /// `params.kelly_fraction`, and clamped by every configured hard
    /// limit (explicit signal size hint, max position size/percent,
    /// available capital).
    pub fn calculate_size(signal: &Signal, params: &RiskParams) -> ExecutionResult<u64> {
        let kelly = Self::kelly_criterion(signal.confidence, params.default_win_loss_ratio);
        let fractional_kelly = kelly * params.kelly_fraction;

        let mut size_usd = params.portfolio_value_usd * fractional_kelly;

        if let Some(suggested) = signal.suggested_size {
            size_usd = size_usd.min(suggested as f64);
        }

        size_usd = size_usd.min(params.max_position_usd);
        size_usd = size_usd.min(params.portfolio_value_usd * params.max_position_percent);
        size_usd = size_usd.min(params.available_capital_usd.max(0.0));

        if size_usd <= 0.0 {
            return Err(ExecutionError::SizingFailed(
                "computed position size is zero or negative".to_string(),
            ));
        }

        Ok(size_usd.round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::{SignalType, TokenPair};

    fn sample_signal(confidence: f64, suggested_size: Option<u64>) -> Signal {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        let mut signal = Signal::new("test".to_string(), SignalType::Buy { pair }, confidence);
        signal.suggested_size = suggested_size;
        signal
    }

    fn sample_params() -> RiskParams {
        RiskParams {
            portfolio_value_usd: 100_000.0,
            available_capital_usd: 100_000.0,
            max_position_usd: 50_000.0,
            max_position_percent: 0.25,
            kelly_fraction: 0.5,
            default_win_loss_ratio: 2.0,
        }
    }

    #[test]
    fn test_kelly_criterion_positive_edge() {
        // p=0.6, b=2.0 -> f* = 0.6 - 0.4/2.0 = 0.4
        let f = PositionSizer::kelly_criterion(0.6, 2.0);
        assert!((f - 0.4).abs() < 1e-9);
    }

    #[test]
    fn test_kelly_criterion_no_edge_clamps_to_zero() {
        // p=0.3, b=1.0 -> f* = 0.3 - 0.7 = -0.4 -> clamped to 0
        let f = PositionSizer::kelly_criterion(0.3, 1.0);
        assert_eq!(f, 0.0);
    }

    #[test]
    fn test_kelly_criterion_zero_win_loss_ratio() {
        assert_eq!(PositionSizer::kelly_criterion(0.9, 0.0), 0.0);
    }

    #[test]
    fn test_calculate_size_respects_max_position_percent() {
        let signal = sample_signal(0.9, None);
        let params = sample_params();

        let size = PositionSizer::calculate_size(&signal, &params).unwrap();
        // 25% of 100k = 25k cap, below the 50k max_position_usd cap.
        assert!(size <= 25_000);
    }

    #[test]
    fn test_calculate_size_respects_suggested_size_cap() {
        let signal = sample_signal(0.95, Some(1_000));
        let params = sample_params();

        let size = PositionSizer::calculate_size(&signal, &params).unwrap();
        assert!(size <= 1_000);
    }

    #[test]
    fn test_calculate_size_zero_confidence_fails() {
        let signal = sample_signal(0.0, None);
        let params = sample_params();

        assert!(PositionSizer::calculate_size(&signal, &params).is_err());
    }

    #[test]
    fn test_calculate_size_respects_available_capital() {
        let signal = sample_signal(0.95, None);
        let params = RiskParams {
            available_capital_usd: 500.0,
            ..sample_params()
        };

        let size = PositionSizer::calculate_size(&signal, &params).unwrap();
        assert!(size <= 500);
    }
}
