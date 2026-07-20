//! Signal ranking.

use solstice_core::types::Signal;

/// Ranks signals for downstream consumption (execution planning picks
/// from the front of the list first).
pub struct SignalRanker;

impl SignalRanker {
    /// Sort signals by confidence, highest first.
    pub fn rank(mut signals: Vec<Signal>) -> Vec<Signal> {
        signals.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        signals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::{SignalType, TokenPair};

    fn signal_with_confidence(confidence: f64) -> Signal {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        Signal::new("test".to_string(), SignalType::Buy { pair }, confidence)
    }

    #[test]
    fn test_rank_sorts_descending_by_confidence() {
        let signals = vec![
            signal_with_confidence(0.5),
            signal_with_confidence(0.9),
            signal_with_confidence(0.7),
        ];

        let ranked = SignalRanker::rank(signals);
        assert_eq!(ranked[0].confidence, 0.9);
        assert_eq!(ranked[1].confidence, 0.7);
        assert_eq!(ranked[2].confidence, 0.5);
    }

    #[test]
    fn test_rank_empty() {
        assert!(SignalRanker::rank(Vec::new()).is_empty());
    }
}
