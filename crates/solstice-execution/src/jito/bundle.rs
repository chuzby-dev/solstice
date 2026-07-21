//! Bundle assembly. A Jito bundle is an ordered list of up to 5 signed
//! transactions that land atomically — either the whole bundle executes, in
//! order, or none of it does. This module is agnostic to what those
//! transactions do: nothing in this workspace currently builds real DEX
//! swap instructions (see the Phase 5 changelog entry for why), so a bundle
//! here is built from whatever already-signed transactions the caller
//! supplies.

use super::error::{JitoError, JitoResult};
use solana_sdk::transaction::VersionedTransaction;

pub const MAX_BUNDLE_TRANSACTIONS: usize = 5;

/// A validated, ordered set of transactions to submit as one atomic Jito
/// bundle.
#[derive(Debug, Clone, Default)]
pub struct Bundle {
    transactions: Vec<VersionedTransaction>,
}

impl Bundle {
    pub fn new() -> Self {
        Bundle {
            transactions: Vec::new(),
        }
    }

    /// Append a transaction, rejecting anything past Jito's 5-transaction
    /// bundle cap rather than silently truncating.
    pub fn add_transaction(&mut self, transaction: VersionedTransaction) -> JitoResult<()> {
        if self.transactions.len() >= MAX_BUNDLE_TRANSACTIONS {
            return Err(JitoError::BundleTooLarge {
                max: MAX_BUNDLE_TRANSACTIONS,
                actual: self.transactions.len() + 1,
            });
        }
        self.transactions.push(transaction);
        Ok(())
    }

    pub fn transactions(&self) -> &[VersionedTransaction] {
        &self.transactions
    }

    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }

    /// Checked before submission: a bundle must have at least one
    /// transaction and no more than [`MAX_BUNDLE_TRANSACTIONS`] (the latter
    /// should be unreachable given `add_transaction`'s own check, but is
    /// re-verified here so submission never sends something Jito would
    /// reject anyway).
    pub(crate) fn validate(&self) -> JitoResult<()> {
        if self.transactions.is_empty() {
            return Err(JitoError::EmptyBundle);
        }
        if self.transactions.len() > MAX_BUNDLE_TRANSACTIONS {
            return Err(JitoError::BundleTooLarge {
                max: MAX_BUNDLE_TRANSACTIONS,
                actual: self.transactions.len(),
            });
        }
        Ok(())
    }
}

/// The landing status of a submitted bundle, as reported by
/// `getBundleStatuses`.
#[derive(Debug, Clone, PartialEq)]
pub enum BundleStatus {
    /// Not yet observed landing (or not found at all — Jito's API doesn't
    /// distinguish "still in flight" from "dropped" until enough time has
    /// passed, so callers should apply their own timeout via
    /// [`super::client::JitoClient::confirm_bundle`]).
    Pending,
    Landed {
        slot: u64,
        confirmation_status: String,
    },
    Failed {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::message::Message;
    use solana_sdk::transaction::Transaction;

    fn dummy_transaction() -> VersionedTransaction {
        VersionedTransaction::from(Transaction::new_unsigned(Message::default()))
    }

    #[test]
    fn test_new_bundle_is_empty() {
        let bundle = Bundle::new();
        assert!(bundle.is_empty());
        assert_eq!(bundle.len(), 0);
    }

    #[test]
    fn test_add_transaction_up_to_cap() {
        let mut bundle = Bundle::new();
        for _ in 0..MAX_BUNDLE_TRANSACTIONS {
            bundle.add_transaction(dummy_transaction()).unwrap();
        }
        assert_eq!(bundle.len(), MAX_BUNDLE_TRANSACTIONS);
    }

    #[test]
    fn test_add_transaction_past_cap_rejected() {
        let mut bundle = Bundle::new();
        for _ in 0..MAX_BUNDLE_TRANSACTIONS {
            bundle.add_transaction(dummy_transaction()).unwrap();
        }
        let result = bundle.add_transaction(dummy_transaction());
        assert!(matches!(result, Err(JitoError::BundleTooLarge { .. })));
        assert_eq!(bundle.len(), MAX_BUNDLE_TRANSACTIONS);
    }

    #[test]
    fn test_validate_rejects_empty_bundle() {
        let bundle = Bundle::new();
        assert!(matches!(bundle.validate(), Err(JitoError::EmptyBundle)));
    }

    #[test]
    fn test_validate_accepts_nonempty_bundle() {
        let mut bundle = Bundle::new();
        bundle.add_transaction(dummy_transaction()).unwrap();
        assert!(bundle.validate().is_ok());
    }
}
