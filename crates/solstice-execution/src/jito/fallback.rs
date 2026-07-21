//! Fallback orchestration (Phase 5.2/5.3): try Jito first, fall back to a
//! direct RPC submission if the bundle is rejected, fails, or doesn't
//! confirm in time.

use super::bundle::{Bundle, BundleStatus};
use super::client::JitoClient;
use super::error::{JitoError, JitoResult};
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;
use solstice_blockchain::SolanaRpcClient;
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Clone, PartialEq)]
pub enum SubmissionMethod {
    Jito,
    Direct,
}

#[derive(Debug, Clone)]
pub struct SubmissionOutcome {
    pub method: SubmissionMethod,
    pub bundle_id: Option<String>,
    pub bundle_status: Option<BundleStatus>,
    pub signatures: Vec<Signature>,
}

/// Try to land `primary_transactions` (plus `tip_transaction`, if any) as
/// one atomic Jito bundle; if the bundle is rejected, fails, or doesn't
/// confirm within `confirm_timeout`, fall back to submitting each of
/// `primary_transactions` directly via `rpc` — **without** the tip
/// transaction, since a direct submission gets no MEV protection and
/// paying the Jito tip for it would just burn SOL for nothing.
///
/// `primary_transactions` and `tip_transaction` must already be signed.
/// This function does not build, price, or sign any transaction — see
/// `docs/CHANGELOG.md`'s Phase 5 entry for why that remains out of scope.
pub async fn submit_with_fallback(
    jito: &JitoClient,
    rpc: &SolanaRpcClient,
    primary_transactions: &[VersionedTransaction],
    tip_transaction: Option<VersionedTransaction>,
    confirm_timeout: Duration,
    poll_interval: Duration,
) -> JitoResult<SubmissionOutcome> {
    if let Some(outcome) = try_jito(
        jito,
        primary_transactions,
        tip_transaction,
        confirm_timeout,
        poll_interval,
    )
    .await
    {
        return Ok(outcome);
    }

    let mut signatures = Vec::with_capacity(primary_transactions.len());
    for transaction in primary_transactions {
        let signature = rpc
            .send_transaction(transaction)
            .await
            .map_err(|e| JitoError::DirectSubmissionFailed(e.to_string()))?;
        signatures.push(signature);
    }

    Ok(SubmissionOutcome {
        method: SubmissionMethod::Direct,
        bundle_id: None,
        bundle_status: None,
        signatures,
    })
}

/// Attempt the Jito path; returns `None` (rather than `Err`) for every
/// failure mode that should fall back to direct submission, so the caller
/// only sees an `Err` for the fallback path's own failures.
async fn try_jito(
    jito: &JitoClient,
    primary_transactions: &[VersionedTransaction],
    tip_transaction: Option<VersionedTransaction>,
    confirm_timeout: Duration,
    poll_interval: Duration,
) -> Option<SubmissionOutcome> {
    if primary_transactions.is_empty() {
        return None;
    }

    let mut bundle = Bundle::new();
    for transaction in primary_transactions {
        if bundle.add_transaction(transaction.clone()).is_err() {
            warn!("too many transactions for a single Jito bundle, skipping Jito path");
            return None;
        }
    }
    if let Some(tip) = tip_transaction {
        if bundle.add_transaction(tip).is_err() {
            warn!("bundle at capacity before tip transaction, skipping Jito path");
            return None;
        }
    }

    let bundle_id = match jito.send_bundle(&bundle).await {
        Ok(id) => id,
        Err(e) => {
            warn!(
                "Jito bundle submission failed ({}), falling back to direct RPC",
                e
            );
            return None;
        }
    };

    match jito
        .confirm_bundle(&bundle_id, confirm_timeout, poll_interval)
        .await
    {
        Ok(status @ BundleStatus::Landed { .. }) => Some(SubmissionOutcome {
            method: SubmissionMethod::Jito,
            bundle_id: Some(bundle_id),
            bundle_status: Some(status),
            signatures: Vec::new(),
        }),
        Ok(status @ BundleStatus::Failed { .. }) => {
            warn!(
                "Jito bundle {} failed ({:?}), falling back to direct RPC",
                bundle_id, status
            );
            None
        }
        Ok(BundleStatus::Pending) | Err(JitoError::ConfirmationTimeout) => {
            warn!(
                "Jito bundle {} did not confirm within the timeout, falling back to direct RPC",
                bundle_id
            );
            None
        }
        Err(e) => {
            warn!(
                "failed polling Jito bundle {} status ({}), falling back to direct RPC",
                bundle_id, e
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jito::client::JitoConfig;
    use solana_sdk::message::Message;
    use solana_sdk::transaction::Transaction;

    fn dummy_transaction() -> VersionedTransaction {
        VersionedTransaction::from(Transaction::new_unsigned(Message::default()))
    }

    fn test_jito_client() -> JitoClient {
        JitoClient::new(JitoConfig::default()).unwrap()
    }

    #[tokio::test]
    async fn test_try_jito_returns_none_for_empty_transactions() {
        let jito = test_jito_client();
        let outcome = try_jito(
            &jito,
            &[],
            None,
            Duration::from_millis(1),
            Duration::from_millis(1),
        )
        .await;
        assert!(outcome.is_none());
    }

    #[tokio::test]
    async fn test_try_jito_returns_none_when_bundle_would_overflow() {
        let jito = test_jito_client();
        // MAX_BUNDLE_TRANSACTIONS primary transactions leave no room for a
        // tip transaction, which should make this bail out before any
        // network call rather than silently dropping the tip.
        let primaries: Vec<VersionedTransaction> = (0
            ..super::super::bundle::MAX_BUNDLE_TRANSACTIONS)
            .map(|_| dummy_transaction())
            .collect();

        let outcome = try_jito(
            &jito,
            &primaries,
            Some(dummy_transaction()),
            Duration::from_millis(1),
            Duration::from_millis(1),
        )
        .await;
        assert!(outcome.is_none());
    }
}
