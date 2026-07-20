//! Signal deduplication.

use chrono::{DateTime, Utc};
use solstice_core::types::Signal;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::Mutex;

struct Seen {
    id: String,
    seen_at: DateTime<Utc>,
}

/// Drops signals whose `id` has been seen within the configured TTL.
pub struct SignalDeduplicator {
    recent: Mutex<VecDeque<Seen>>,
    ttl: Duration,
}

impl SignalDeduplicator {
    pub fn new(ttl: Duration) -> Self {
        SignalDeduplicator {
            recent: Mutex::new(VecDeque::new()),
            ttl,
        }
    }

    /// Filter `signals` down to those not seen within the TTL, recording
    /// the survivors as seen.
    pub async fn deduplicate(&self, signals: Vec<Signal>) -> Vec<Signal> {
        let mut recent = self.recent.lock().await;

        let now = Utc::now();
        let ttl = chrono::Duration::from_std(self.ttl).unwrap_or(chrono::Duration::zero());
        recent.retain(|s| now.signed_duration_since(s.seen_at) < ttl);

        let mut kept = Vec::with_capacity(signals.len());
        for signal in signals {
            let is_duplicate = recent.iter().any(|s| s.id == signal.id);
            if !is_duplicate {
                recent.push_back(Seen {
                    id: signal.id.clone(),
                    seen_at: now,
                });
                kept.push(signal);
            }
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use solstice_core::types::{SignalType, TokenPair};

    fn sample_signal() -> Signal {
        let pair = TokenPair::new(Pubkey::new_unique(), Pubkey::new_unique());
        Signal::new("test".to_string(), SignalType::Buy { pair }, 0.9)
    }

    #[tokio::test]
    async fn test_first_occurrence_kept() {
        let dedup = SignalDeduplicator::new(Duration::from_secs(60));
        let signal = sample_signal();
        let result = dedup.deduplicate(vec![signal]).await;
        assert_eq!(result.len(), 1);
    }

    #[tokio::test]
    async fn test_duplicate_dropped() {
        let dedup = SignalDeduplicator::new(Duration::from_secs(60));
        let signal = sample_signal();

        let first = dedup.deduplicate(vec![signal.clone()]).await;
        let second = dedup.deduplicate(vec![signal]).await;

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 0);
    }

    #[tokio::test]
    async fn test_duplicate_allowed_after_ttl_expires() {
        let dedup = SignalDeduplicator::new(Duration::from_millis(10));
        let signal = sample_signal();

        let first = dedup.deduplicate(vec![signal.clone()]).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let second = dedup.deduplicate(vec![signal]).await;

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
    }
}
