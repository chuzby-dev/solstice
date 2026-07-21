//! Solana RPC client with connection pooling and failover.

use crate::accounts::{AccountInfo, BatchAccountResult};
use crate::error::{BlockchainError, BlockchainResult};
use crate::types::{
    EndpointHealth, RpcClientConfig, RpcEndpointConfig, TransactionConfirmation, TransactionStatus,
};
use chrono::Utc;
use solana_client::nonblocking::rpc_client::RpcClient as NonblockingRpcClient;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::Transaction;
use solana_transaction_status_client_types::TransactionConfirmationStatus as SolanaConfirmationStatus;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{debug, error};

/// Solana RPC client with failover and connection pooling support.
pub struct SolanaRpcClient {
    config: Arc<RpcClientConfig>,
    endpoint_health: Arc<RwLock<HashMap<String, EndpointHealth>>>,
    selected_endpoint: Arc<RwLock<Option<String>>>,
}

impl SolanaRpcClient {
    /// Create a new RPC client from configuration.
    pub fn new(config: RpcClientConfig) -> BlockchainResult<Self> {
        if config.endpoints.is_empty() {
            return Err(BlockchainError::NoEndpoints);
        }

        let mut health = HashMap::new();
        for endpoint in &config.endpoints {
            health.insert(
                endpoint.url.clone(),
                EndpointHealth::new(endpoint.url.clone()),
            );
        }

        Ok(SolanaRpcClient {
            config: Arc::new(config),
            endpoint_health: Arc::new(RwLock::new(health)),
            selected_endpoint: Arc::new(RwLock::new(None)),
        })
    }

    /// Create a new RPC client with default configuration and specified endpoints.
    pub fn with_endpoints(urls: Vec<String>) -> BlockchainResult<Self> {
        let config = RpcClientConfig {
            endpoints: urls.into_iter().map(RpcEndpointConfig::new).collect(),
            ..Default::default()
        };
        Self::new(config)
    }

    /// Get the current active endpoint URL.
    pub fn get_active_endpoint(&self) -> BlockchainResult<String> {
        let selected = self
            .selected_endpoint
            .read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?
            .clone();

        if let Some(endpoint) = selected {
            return Ok(endpoint);
        }

        // Select best endpoint based on health
        self.select_best_endpoint()
    }

    /// Select the best endpoint based on health metrics.
    fn select_best_endpoint(&self) -> BlockchainResult<String> {
        let health = self
            .endpoint_health
            .read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        // Filter to healthy endpoints
        let healthy: Vec<_> = self
            .config
            .endpoints
            .iter()
            .filter(|ep| health.get(&ep.url).map(|h| h.is_healthy).unwrap_or(true))
            .collect();

        if healthy.is_empty() {
            error!("No healthy endpoints available");
            return Err(BlockchainError::AllEndpointsFailed);
        }

        // Select highest priority healthy endpoint
        let best = healthy
            .into_iter()
            .max_by_key(|ep| ep.priority)
            .ok_or(BlockchainError::AllEndpointsFailed)?;

        let endpoint_url = best.url.clone();
        drop(health);

        // Update selected endpoint
        *self
            .selected_endpoint
            .write()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))? =
            Some(endpoint_url.clone());

        debug!("Selected RPC endpoint: {}", endpoint_url);
        Ok(endpoint_url)
    }

    /// Record a successful RPC call.
    pub fn record_success(&self, endpoint: &str, latency_ms: f64) -> BlockchainResult<()> {
        let mut health = self
            .endpoint_health
            .write()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        if let Some(ep_health) = health.get_mut(endpoint) {
            ep_health.record_success(latency_ms);
        }
        Ok(())
    }

    /// Record a failed RPC call.
    pub fn record_error(&self, endpoint: &str, error: String) -> BlockchainResult<()> {
        let mut health = self
            .endpoint_health
            .write()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        if let Some(ep_health) = health.get_mut(endpoint) {
            ep_health.record_error(error);
        }

        // Clear selected endpoint so next call selects a new one
        *self
            .selected_endpoint
            .write()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))? = None;

        Ok(())
    }

    /// Get health status of all endpoints.
    pub fn get_endpoint_health(&self) -> BlockchainResult<Vec<EndpointHealth>> {
        let health = self
            .endpoint_health
            .read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        Ok(health.values().cloned().collect())
    }

    /// Check if endpoint is healthy.
    pub fn is_endpoint_healthy(&self, endpoint: &str) -> BlockchainResult<bool> {
        let health = self
            .endpoint_health
            .read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        Ok(health.get(endpoint).map(|h| h.is_healthy).unwrap_or(false))
    }

    /// Get active endpoint health status.
    pub fn get_active_endpoint_health(&self) -> BlockchainResult<EndpointHealth> {
        let endpoint = self.get_active_endpoint()?;
        let health = self
            .endpoint_health
            .read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        health
            .get(&endpoint)
            .cloned()
            .ok_or(BlockchainError::ConnectionError(
                "Endpoint health not found".to_string(),
            ))
    }

    fn build_rpc_client(&self, endpoint: &str) -> NonblockingRpcClient {
        let timeout = self
            .config
            .endpoints
            .iter()
            .find(|e| e.url == endpoint)
            .map(|e| Duration::from_secs(e.timeout_secs))
            .unwrap_or(Duration::from_secs(30));

        NonblockingRpcClient::new_with_timeout(endpoint.to_string(), timeout)
    }

    /// Fetch a single account's state, trying each endpoint in priority
    /// order (with health tracking) until one succeeds or all fail.
    pub async fn get_account(&self, pubkey: &Pubkey) -> BlockchainResult<AccountInfo> {
        let result = self
            .get_multiple_accounts(std::slice::from_ref(pubkey))
            .await?;
        result
            .accounts
            .into_iter()
            .next()
            .ok_or_else(|| BlockchainError::AccountNotFound(pubkey.to_string()))
    }

    /// Fetch multiple accounts in a single RPC round trip. Missing accounts
    /// are reported in [`BatchAccountResult::failed`] rather than causing
    /// the whole call to fail.
    pub async fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> BlockchainResult<BatchAccountResult> {
        if pubkeys.is_empty() {
            return Ok(BatchAccountResult::new());
        }

        let max_attempts = self.config.max_retries.max(1);
        let overall_start = Instant::now();
        let mut last_error: Option<String> = None;

        for _ in 0..max_attempts {
            let endpoint = self.get_active_endpoint()?;
            let rpc = self.build_rpc_client(&endpoint);
            let call_start = Instant::now();

            match rpc.get_multiple_accounts(pubkeys).await {
                Ok(accounts) => {
                    let latency_ms = call_start.elapsed().as_secs_f64() * 1000.0;
                    self.record_success(&endpoint, latency_ms)?;

                    let mut result = BatchAccountResult::new();
                    for (pubkey, maybe_account) in pubkeys.iter().zip(accounts) {
                        match maybe_account {
                            Some(account) => result
                                .accounts
                                .push(AccountInfo::from_solana_account(*pubkey, account)),
                            None => result
                                .failed
                                .push((*pubkey, "account not found".to_string())),
                        }
                    }
                    result.elapsed_ms = overall_start.elapsed().as_millis() as u64;
                    return Ok(result);
                }
                Err(e) => {
                    let message = e.to_string();
                    self.record_error(&endpoint, message.clone())?;
                    last_error = Some(message);
                }
            }
        }

        Err(BlockchainError::RpcError(
            last_error.unwrap_or_else(|| "all RPC attempts failed".to_string()),
        ))
    }

    /// Fetch an account's SOL balance in lamports, trying each endpoint in
    /// priority order. Unlike [`Self::get_account`], a never-funded address
    /// returns `Ok(0)` rather than [`BlockchainError::AccountNotFound`] --
    /// the natural behavior for "what's this wallet's balance," where an
    /// empty wallet is a normal state, not an error.
    pub async fn get_balance(&self, pubkey: &Pubkey) -> BlockchainResult<u64> {
        let max_attempts = self.config.max_retries.max(1);
        let mut last_error: Option<String> = None;

        for _ in 0..max_attempts {
            let endpoint = self.get_active_endpoint()?;
            let rpc = self.build_rpc_client(&endpoint);
            let call_start = Instant::now();

            match rpc.get_balance(pubkey).await {
                Ok(lamports) => {
                    let latency_ms = call_start.elapsed().as_secs_f64() * 1000.0;
                    self.record_success(&endpoint, latency_ms)?;
                    return Ok(lamports);
                }
                Err(e) => {
                    let message = e.to_string();
                    self.record_error(&endpoint, message.clone())?;
                    last_error = Some(message);
                }
            }
        }

        Err(BlockchainError::RpcError(
            last_error.unwrap_or_else(|| "all RPC attempts failed".to_string()),
        ))
    }

    /// Fetch a recent blockhash, trying each endpoint in priority order.
    pub async fn get_latest_blockhash(&self) -> BlockchainResult<Hash> {
        let max_attempts = self.config.max_retries.max(1);
        let mut last_error: Option<String> = None;

        for _ in 0..max_attempts {
            let endpoint = self.get_active_endpoint()?;
            let rpc = self.build_rpc_client(&endpoint);
            let call_start = Instant::now();

            match rpc.get_latest_blockhash().await {
                Ok(hash) => {
                    let latency_ms = call_start.elapsed().as_secs_f64() * 1000.0;
                    self.record_success(&endpoint, latency_ms)?;
                    return Ok(hash);
                }
                Err(e) => {
                    let message = e.to_string();
                    self.record_error(&endpoint, message.clone())?;
                    last_error = Some(message);
                }
            }
        }

        Err(BlockchainError::RpcError(
            last_error.unwrap_or_else(|| "all RPC attempts failed".to_string()),
        ))
    }

    /// Submit a signed transaction, trying each endpoint in priority order
    /// until one accepts it or all fail. This only submits the transaction
    /// — it does not wait for confirmation; callers that need that should
    /// follow up with [`Self::confirm_transaction`].
    pub async fn send_transaction(&self, transaction: &Transaction) -> BlockchainResult<Signature> {
        let max_attempts = self.config.max_retries.max(1);
        let mut last_error: Option<String> = None;

        for _ in 0..max_attempts {
            let endpoint = self.get_active_endpoint()?;
            let rpc = self.build_rpc_client(&endpoint);
            let call_start = Instant::now();

            match rpc.send_transaction(transaction).await {
                Ok(signature) => {
                    let latency_ms = call_start.elapsed().as_secs_f64() * 1000.0;
                    self.record_success(&endpoint, latency_ms)?;
                    return Ok(signature);
                }
                Err(e) => {
                    let message = e.to_string();
                    self.record_error(&endpoint, message.clone())?;
                    last_error = Some(message);
                }
            }
        }

        Err(BlockchainError::TransactionFailed(
            last_error.unwrap_or_else(|| "all RPC attempts failed".to_string()),
        ))
    }

    /// Poll `getSignatureStatuses` for `signature` until it confirms, fails
    /// on-chain, or `timeout` elapses. Populates the pre-existing
    /// [`TransactionConfirmation`]/[`TransactionStatus`] types, which
    /// nothing previously produced from a real RPC call.
    pub async fn confirm_transaction(
        &self,
        signature: &Signature,
        timeout: Duration,
        poll_interval: Duration,
    ) -> BlockchainResult<TransactionConfirmation> {
        let deadline = Instant::now() + timeout;

        loop {
            let endpoint = self.get_active_endpoint()?;
            let rpc = self.build_rpc_client(&endpoint);

            match rpc
                .get_signature_statuses(std::slice::from_ref(signature))
                .await
            {
                Ok(response) => {
                    self.record_success(&endpoint, 0.0)?;
                    if let Some(Some(status)) = response.value.into_iter().next() {
                        if let Some(err) = status.err {
                            return Ok(TransactionConfirmation {
                                signature: *signature,
                                status: TransactionStatus::Failed,
                                slot: Some(status.slot),
                                error: Some(err.to_string()),
                                timestamp: Utc::now(),
                            });
                        }

                        let is_finalized = matches!(
                            status.confirmation_status,
                            Some(SolanaConfirmationStatus::Finalized)
                        );
                        let is_confirmed = is_finalized
                            || matches!(
                                status.confirmation_status,
                                Some(SolanaConfirmationStatus::Confirmed)
                            );

                        if is_confirmed {
                            return Ok(TransactionConfirmation {
                                signature: *signature,
                                status: if is_finalized {
                                    TransactionStatus::Finalized
                                } else {
                                    TransactionStatus::Confirmed
                                },
                                slot: Some(status.slot),
                                error: None,
                                timestamp: Utc::now(),
                            });
                        }
                    }
                }
                Err(e) => {
                    self.record_error(&endpoint, e.to_string())?;
                }
            }

            if Instant::now() >= deadline {
                return Err(BlockchainError::TransactionTimeout);
            }
            tokio::time::sleep(poll_interval).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_multiple_accounts_empty_input() {
        let config = RpcClientConfig {
            endpoints: vec![RpcEndpointConfig::new("http://localhost:8899".to_string())],
            ..Default::default()
        };
        let client = SolanaRpcClient::new(config).unwrap();

        let result = client.get_multiple_accounts(&[]).await.unwrap();
        assert!(result.accounts.is_empty());
        assert!(result.failed.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires a live, reachable Solana RPC endpoint"]
    async fn test_get_account_live() {
        // System program account; always exists on any real cluster.
        let system_program = Pubkey::default();
        let client =
            SolanaRpcClient::with_endpoints(
                vec!["https://api.mainnet-beta.solana.com".to_string()],
            )
            .unwrap();

        let account = client.get_account(&system_program).await.unwrap();
        assert_eq!(account.address, system_program);
    }

    #[tokio::test]
    #[ignore = "requires a live, reachable Solana RPC endpoint"]
    async fn test_get_latest_blockhash_live() {
        let client =
            SolanaRpcClient::with_endpoints(
                vec!["https://api.mainnet-beta.solana.com".to_string()],
            )
            .unwrap();

        let hash = client.get_latest_blockhash().await.unwrap();
        assert_ne!(hash, Hash::default());
    }

    #[tokio::test]
    async fn test_get_latest_blockhash_fails_cleanly_when_unreachable() {
        // Nothing listens on port 1, so this fails fast (connection
        // refused) rather than waiting out a real timeout.
        let client =
            SolanaRpcClient::with_endpoints(vec!["http://127.0.0.1:1".to_string()]).unwrap();
        let result = client.get_latest_blockhash().await;
        assert!(matches!(result, Err(BlockchainError::RpcError(_))));
    }

    #[tokio::test]
    async fn test_get_balance_fails_cleanly_when_unreachable() {
        let client =
            SolanaRpcClient::with_endpoints(vec!["http://127.0.0.1:1".to_string()]).unwrap();
        let result = client.get_balance(&Pubkey::default()).await;
        assert!(matches!(result, Err(BlockchainError::RpcError(_))));
    }

    #[tokio::test]
    #[ignore = "requires a live, reachable Solana RPC endpoint"]
    async fn test_get_balance_live() {
        // System program: always exists, balance is well-defined (zero or
        // more), on any real cluster.
        let client =
            SolanaRpcClient::with_endpoints(
                vec!["https://api.mainnet-beta.solana.com".to_string()],
            )
            .unwrap();
        // Just confirming the call succeeds and returns a sane type;
        // the exact balance isn't asserted since it can change.
        let _balance = client.get_balance(&Pubkey::default()).await.unwrap();
    }

    #[tokio::test]
    async fn test_send_transaction_fails_cleanly_when_unreachable() {
        let client =
            SolanaRpcClient::with_endpoints(vec!["http://127.0.0.1:1".to_string()]).unwrap();
        let transaction = Transaction::new_unsigned(solana_sdk::message::Message::default());
        let result = client.send_transaction(&transaction).await;
        assert!(matches!(result, Err(BlockchainError::TransactionFailed(_))));
    }

    #[tokio::test]
    async fn test_confirm_transaction_times_out_when_unreachable() {
        let client =
            SolanaRpcClient::with_endpoints(vec!["http://127.0.0.1:1".to_string()]).unwrap();
        let result = client
            .confirm_transaction(
                &Signature::default(),
                Duration::from_millis(50),
                Duration::from_millis(10),
            )
            .await;
        assert!(matches!(result, Err(BlockchainError::TransactionTimeout)));
    }

    /// The full, previously-never-tested "sign, submit, confirm" pipeline,
    /// run for real against Solana's public devnet -- not mainnet, and not
    /// a real financial asset: devnet SOL is faucet-issued test currency
    /// with no monetary value. This is the first time this codebase has
    /// ever actually submitted a transaction to any network; everything
    /// before this was either paper-simulated or a read-only RPC call.
    ///
    /// Set `DEVNET_TEST_KEYPAIR` to the path of an already-funded keypair
    /// JSON file (e.g. via `cargo run -p solstice-blockchain --example
    /// gen_devnet_keypair`, funded manually at faucet.solana.com) to reuse
    /// it instead of requesting a fresh airdrop -- useful in environments
    /// where the devnet faucet is IP-rate-limited (confirmed to be the
    /// case for this project's dev sandbox; this path was verified working
    /// there: real transaction, real confirmation, see the Phase 10
    /// changelog entry). With no env var set, a fresh keypair is generated
    /// and airdropped to, as before.
    #[tokio::test]
    #[ignore = "requires network access to Solana devnet (and its faucet, unless DEVNET_TEST_KEYPAIR is set)"]
    async fn test_sign_submit_confirm_pipeline_on_devnet() {
        use solana_sdk::signature::{Keypair, Signer};
        // See the same `#[allow(deprecated)]` rationale in
        // `solstice-execution::jito::tip`: solana-sdk 2.x still ships this,
        // and pulling in `solana-system-interface` for one call isn't
        // worth it here.
        #[allow(deprecated)]
        use solana_sdk::system_instruction;

        const DEVNET_RPC: &str = "https://api.devnet.solana.com";

        let payer = if let Ok(path) = std::env::var("DEVNET_TEST_KEYPAIR") {
            let bytes: Vec<u8> = serde_json::from_str(
                &std::fs::read_to_string(&path).expect("failed to read DEVNET_TEST_KEYPAIR file"),
            )
            .expect("DEVNET_TEST_KEYPAIR file is not a valid JSON byte array");
            Keypair::try_from(bytes.as_slice()).expect("invalid keypair bytes")
        } else {
            // Ephemeral, throwaway keypair -- generated fresh, funded with
            // free faucet SOL, and discarded when the test ends.
            let payer = Keypair::new();

            let raw_rpc =
                solana_client::nonblocking::rpc_client::RpcClient::new(DEVNET_RPC.to_string());
            let airdrop_sig = raw_rpc
                .request_airdrop(&payer.pubkey(), 1_000_000_000) // 1 devnet SOL
                .await
                .expect("devnet airdrop request failed");

            let mut airdropped = false;
            for _ in 0..30 {
                if raw_rpc
                    .confirm_transaction(&airdrop_sig)
                    .await
                    .unwrap_or(false)
                {
                    airdropped = true;
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            assert!(airdropped, "devnet airdrop did not confirm in time");
            payer
        };

        let client = SolanaRpcClient::with_endpoints(vec![DEVNET_RPC.to_string()]).unwrap();

        // A harmless self-transfer of 1 lamport -- proves the sign/submit
        // pipeline works without needing any real swap or DEX involved.
        let instruction = system_instruction::transfer(&payer.pubkey(), &payer.pubkey(), 1);
        let blockhash = client.get_latest_blockhash().await.unwrap();
        let transaction = crate::transaction::TransactionBuilder::new()
            .payer(payer.pubkey())
            .add_instruction(instruction)
            .build_and_sign(blockhash.to_bytes(), &[&payer])
            .unwrap();

        let signature = client.send_transaction(&transaction).await.unwrap();

        let confirmation = client
            .confirm_transaction(&signature, Duration::from_secs(30), Duration::from_secs(2))
            .await
            .unwrap();

        assert!(
            confirmation.is_confirmed(),
            "transaction did not confirm: {confirmation:?}"
        );
        assert!(!confirmation.is_failed());
    }

    #[test]
    fn test_client_creation() {
        let config = RpcClientConfig {
            endpoints: vec![RpcEndpointConfig::new("http://localhost:8899".to_string())],
            ..Default::default()
        };

        let client = SolanaRpcClient::new(config).unwrap();
        assert!(client.get_active_endpoint().is_ok());
    }

    #[test]
    fn test_no_endpoints_error() {
        let config = RpcClientConfig::default();
        let result = SolanaRpcClient::new(config);
        assert!(matches!(result, Err(BlockchainError::NoEndpoints)));
    }

    #[test]
    fn test_endpoint_selection() {
        let config = RpcClientConfig {
            endpoints: vec![
                RpcEndpointConfig::new("http://endpoint1:8899".to_string()),
                RpcEndpointConfig::new("http://endpoint2:8899".to_string()),
            ],
            ..Default::default()
        };

        let client = SolanaRpcClient::new(config).unwrap();
        let endpoint = client.get_active_endpoint().unwrap();
        assert!(!endpoint.is_empty());
    }

    #[test]
    fn test_error_recording() {
        let config = RpcClientConfig {
            endpoints: vec![RpcEndpointConfig::new("http://localhost:8899".to_string())],
            ..Default::default()
        };

        let client = SolanaRpcClient::new(config).unwrap();
        let endpoint = client.get_active_endpoint().unwrap();

        // Record error
        client
            .record_error(&endpoint, "test error".to_string())
            .unwrap();

        // Endpoint should be marked unhealthy after enough errors
        let health = client.get_active_endpoint_health().unwrap();
        assert_eq!(health.consecutive_errors, 1);
    }

    #[test]
    fn test_success_recording() {
        let config = RpcClientConfig {
            endpoints: vec![RpcEndpointConfig::new("http://localhost:8899".to_string())],
            ..Default::default()
        };

        let client = SolanaRpcClient::new(config).unwrap();
        let endpoint = client.get_active_endpoint().unwrap();

        // Record success
        client.record_success(&endpoint, 50.0).unwrap();

        let health = client.get_active_endpoint_health().unwrap();
        assert_eq!(health.consecutive_errors, 0);
        assert!(health.avg_latency_ms > 0.0);
    }

    #[test]
    fn test_get_health_status() {
        let config = RpcClientConfig {
            endpoints: vec![
                RpcEndpointConfig::new("http://endpoint1:8899".to_string()),
                RpcEndpointConfig::new("http://endpoint2:8899".to_string()),
            ],
            ..Default::default()
        };

        let client = SolanaRpcClient::new(config).unwrap();
        let healths = client.get_endpoint_health().unwrap();
        assert_eq!(healths.len(), 2);
    }
}
