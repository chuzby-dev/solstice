//! Blockchain-specific types.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use solana_sdk::signature::Signature;

/// Configuration for RPC endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEndpointConfig {
    /// RPC endpoint URL.
    pub url: String,
    /// Priority weight for this endpoint (higher = preferred).
    pub priority: u32,
    /// Connection timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum concurrent requests.
    pub max_concurrent: usize,
}

impl RpcEndpointConfig {
    /// Create a new RPC endpoint configuration.
    pub fn new(url: String) -> Self {
        RpcEndpointConfig {
            url,
            priority: 100,
            timeout_secs: 30,
            max_concurrent: 100,
        }
    }

    /// Set priority for this endpoint.
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Set timeout for this endpoint.
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }
}

/// RPC client configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcClientConfig {
    /// List of RPC endpoints.
    pub endpoints: Vec<RpcEndpointConfig>,
    /// Maximum retries for failed requests.
    pub max_retries: u32,
    /// Initial retry backoff in milliseconds.
    pub retry_backoff_ms: u64,
    /// Maximum retry backoff in milliseconds.
    pub max_retry_backoff_ms: u64,
    /// Enable caching of responses.
    pub cache_enabled: bool,
    /// Cache TTL in seconds.
    pub cache_ttl_secs: u64,
}

impl Default for RpcClientConfig {
    fn default() -> Self {
        RpcClientConfig {
            endpoints: vec![],
            max_retries: 3,
            retry_backoff_ms: 100,
            max_retry_backoff_ms: 5000,
            cache_enabled: true,
            cache_ttl_secs: 5,
        }
    }
}

/// Transaction confirmation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionStatus {
    /// Transaction is pending (not yet confirmed).
    Pending,
    /// Transaction has been processed and confirmed.
    Confirmed,
    /// Transaction failed.
    Failed,
    /// Transaction finalized (cannot be rolled back).
    Finalized,
}

/// Transaction confirmation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionConfirmation {
    /// Transaction signature.
    pub signature: Signature,
    /// Confirmation status.
    pub status: TransactionStatus,
    /// Slot where transaction was confirmed.
    pub slot: Option<u64>,
    /// Error message if transaction failed.
    pub error: Option<String>,
    /// Timestamp of confirmation check.
    pub timestamp: DateTime<Utc>,
}

impl TransactionConfirmation {
    /// Create a pending confirmation.
    pub fn pending(signature: Signature) -> Self {
        TransactionConfirmation {
            signature,
            status: TransactionStatus::Pending,
            slot: None,
            error: None,
            timestamp: Utc::now(),
        }
    }

    /// Check if transaction is confirmed or finalized.
    pub fn is_confirmed(&self) -> bool {
        matches!(
            self.status,
            TransactionStatus::Confirmed | TransactionStatus::Finalized
        )
    }

    /// Check if transaction failed.
    pub fn is_failed(&self) -> bool {
        self.status == TransactionStatus::Failed
    }
}

/// RPC endpoint health status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointHealth {
    /// Endpoint URL.
    pub url: String,
    /// Is endpoint healthy.
    pub is_healthy: bool,
    /// Average latency in milliseconds.
    pub avg_latency_ms: f64,
    /// Error rate (0.0 to 1.0).
    pub error_rate: f64,
    /// Consecutive errors.
    pub consecutive_errors: u32,
    /// Last error message.
    pub last_error: Option<String>,
    /// Last health check timestamp.
    pub last_check: DateTime<Utc>,
}

impl EndpointHealth {
    /// Create new endpoint health status.
    pub fn new(url: String) -> Self {
        EndpointHealth {
            url,
            is_healthy: true,
            avg_latency_ms: 0.0,
            error_rate: 0.0,
            consecutive_errors: 0,
            last_error: None,
            last_check: Utc::now(),
        }
    }

    /// Mark endpoint as having an error.
    pub fn record_error(&mut self, error: String) {
        self.consecutive_errors += 1;
        self.last_error = Some(error);
        self.is_healthy = self.consecutive_errors < 5;
        self.error_rate = (self.consecutive_errors as f64) / 10.0;
    }

    /// Mark endpoint as successful.
    pub fn record_success(&mut self, latency_ms: f64) {
        self.consecutive_errors = 0;
        self.last_error = None;
        self.is_healthy = true;
        self.avg_latency_ms = (self.avg_latency_ms + latency_ms) / 2.0;
        self.error_rate = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_endpoint_config() {
        let config = RpcEndpointConfig::new("http://localhost:8899".to_string())
            .with_priority(200)
            .with_timeout(60);

        assert_eq!(config.priority, 200);
        assert_eq!(config.timeout_secs, 60);
    }

    #[test]
    fn test_transaction_confirmation_status() {
        let sig = Signature::default();
        let mut conf = TransactionConfirmation::pending(sig);

        assert!(!conf.is_confirmed());
        assert!(!conf.is_failed());

        conf.status = TransactionStatus::Confirmed;
        assert!(conf.is_confirmed());
    }

    #[test]
    fn test_endpoint_health_tracking() {
        let mut health = EndpointHealth::new("http://localhost:8899".to_string());
        assert!(health.is_healthy);

        health.record_success(100.0);
        assert_eq!(health.avg_latency_ms, 100.0);
        assert_eq!(health.consecutive_errors, 0);

        for _ in 0..5 {
            health.record_error("test error".to_string());
        }
        assert!(!health.is_healthy);
    }
}
