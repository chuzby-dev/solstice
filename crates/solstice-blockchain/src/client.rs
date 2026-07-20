//! Solana RPC client with connection pooling and failover.

use crate::error::{BlockchainError, BlockchainResult};
use crate::types::{RpcClientConfig, RpcEndpointConfig, EndpointHealth, TransactionConfirmation, TransactionStatus};
use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use tracing::{debug, warn, error};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use chrono::Utc;

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
            health.insert(endpoint.url.clone(), EndpointHealth::new(endpoint.url.clone()));
        }

        Ok(SolanaRpcClient {
            config: Arc::new(config),
            endpoint_health: Arc::new(RwLock::new(health)),
            selected_endpoint: Arc::new(RwLock::new(None)),
        })
    }

    /// Create a new RPC client with default configuration and specified endpoints.
    pub fn with_endpoints(urls: Vec<String>) -> BlockchainResult<Self> {
        let mut config = RpcClientConfig::default();
        config.endpoints = urls
            .into_iter()
            .map(RpcEndpointConfig::new)
            .collect();
        Self::new(config)
    }

    /// Get the current active endpoint URL.
    pub fn get_active_endpoint(&self) -> BlockchainResult<String> {
        let selected = self.selected_endpoint.read()
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
        let health = self.endpoint_health.read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        // Filter to healthy endpoints
        let healthy: Vec<_> = self.config.endpoints
            .iter()
            .filter(|ep| {
                health.get(&ep.url)
                    .map(|h| h.is_healthy)
                    .unwrap_or(true)
            })
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
        *self.selected_endpoint.write()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?
            = Some(endpoint_url.clone());

        debug!("Selected RPC endpoint: {}", endpoint_url);
        Ok(endpoint_url)
    }

    /// Record a successful RPC call.
    pub fn record_success(&self, endpoint: &str, latency_ms: f64) -> BlockchainResult<()> {
        let mut health = self.endpoint_health.write()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        if let Some(ep_health) = health.get_mut(endpoint) {
            ep_health.record_success(latency_ms);
        }
        Ok(())
    }

    /// Record a failed RPC call.
    pub fn record_error(&self, endpoint: &str, error: String) -> BlockchainResult<()> {
        let mut health = self.endpoint_health.write()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        if let Some(ep_health) = health.get_mut(endpoint) {
            ep_health.record_error(error);
        }

        // Clear selected endpoint so next call selects a new one
        *self.selected_endpoint.write()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?
            = None;

        Ok(())
    }

    /// Get health status of all endpoints.
    pub fn get_endpoint_health(&self) -> BlockchainResult<Vec<EndpointHealth>> {
        let health = self.endpoint_health.read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        Ok(health.values().cloned().collect())
    }

    /// Check if endpoint is healthy.
    pub fn is_endpoint_healthy(&self, endpoint: &str) -> BlockchainResult<bool> {
        let health = self.endpoint_health.read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        Ok(health.get(endpoint)
            .map(|h| h.is_healthy)
            .unwrap_or(false))
    }

    /// Get active endpoint health status.
    pub fn get_active_endpoint_health(&self) -> BlockchainResult<EndpointHealth> {
        let endpoint = self.get_active_endpoint()?;
        let health = self.endpoint_health.read()
            .map_err(|_| BlockchainError::ConnectionError("Lock poisoned".to_string()))?;

        health.get(&endpoint)
            .cloned()
            .ok_or(BlockchainError::ConnectionError("Endpoint health not found".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let config = RpcClientConfig {
            endpoints: vec![
                RpcEndpointConfig::new("http://localhost:8899".to_string()),
            ],
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
            endpoints: vec![
                RpcEndpointConfig::new("http://localhost:8899".to_string()),
            ],
            ..Default::default()
        };

        let client = SolanaRpcClient::new(config).unwrap();
        let endpoint = client.get_active_endpoint().unwrap();

        // Record error
        client.record_error(&endpoint, "test error".to_string()).unwrap();

        // Endpoint should be marked unhealthy after enough errors
        let health = client.get_active_endpoint_health().unwrap();
        assert_eq!(health.consecutive_errors, 1);
    }

    #[test]
    fn test_success_recording() {
        let config = RpcClientConfig {
            endpoints: vec![
                RpcEndpointConfig::new("http://localhost:8899".to_string()),
            ],
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
