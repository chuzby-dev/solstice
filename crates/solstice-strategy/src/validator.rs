//! Signal validation.

use crate::error::{StrategyError, StrategyResult};
use solstice_core::types::Signal;

/// Validates signals before they're forwarded to execution.
#[derive(Debug, Clone, Copy)]
pub struct SignalValidator {
    pub min_confidence: f64,
}

impl SignalValidator {
    pub fn new(min_confidence: f64) -> Self {
        SignalValidator { min_confidence }
    }

    pub fn validate(&self, signal: &Signal) -> StrategyResult<()> {
        if !(0.0..=1.0).contains(&signal.confidence) {
            return Err(StrategyError::InvalidSignal(format!(
                "confidence {} out of range [0.0, 1.0]",
                signal.confidence
            )));
        }

        if signal.confidence < self.min_confidence {
            return Err(StrategyError::InvalidSignal(format!(
                "confidence {} below minimum {}",
                signal.confidence, self.min_confidence
            )));
        }

        if let Some(size) = signal.suggested_size {
            if size == 0 {
                return Err(StrategyError::InvalidSignal(
                    "suggested_size is zero".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Filter a batch of signals, keeping only valid ones (invalid signals
    /// are dropped and logged, not propagated as an error, since one bad
    /// signal from one strategy shouldn't block the rest of the batch).
    pub fn filter_valid(&self, signals: Vec<Signal>) -> Vec<Signal> {
        signals
            .into_iter()
            .filter(|s| match self.validate(s) {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!("Dropping invalid signal from {}: {}", s.strategy, e);
                    false
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::{Signal, SignalType, TokenPair};

    fn sample_signal(confidence: f64) -> Signal {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        Signal::new("test".to_string(), SignalType::Buy { pair }, confidence)
    }

    #[test]
    fn test_valid_signal_passes() {
        let validator = SignalValidator::new(0.5);
        assert!(validator.validate(&sample_signal(0.8)).is_ok());
    }

    #[test]
    fn test_low_confidence_rejected() {
        let validator = SignalValidator::new(0.5);
        // Signal::new clamps confidence into [0,1], so 0.3 stays 0.3.
        assert!(validator.validate(&sample_signal(0.3)).is_err());
    }

    #[test]
    fn test_zero_suggested_size_rejected() {
        let validator = SignalValidator::new(0.5);
        let mut signal = sample_signal(0.9);
        signal.suggested_size = Some(0);
        assert!(validator.validate(&signal).is_err());
    }

    #[test]
    fn test_filter_valid_drops_invalid_keeps_valid() {
        let validator = SignalValidator::new(0.5);
        let signals = vec![sample_signal(0.9), sample_signal(0.1)];
        let filtered = validator.filter_valid(signals);
        assert_eq!(filtered.len(), 1);
    }
}
