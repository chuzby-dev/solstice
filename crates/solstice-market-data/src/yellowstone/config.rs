//! Connection and tuning configuration for the Yellowstone gRPC adapter.

use std::time::Duration;

/// Configuration for connecting to one or more Yellowstone gRPC endpoints.
#[derive(Debug, Clone)]
pub struct YellowstoneConfig {
    /// Endpoints tried first, in order, on every (re)connect attempt.
    pub primary_endpoints: Vec<String>,
    /// Endpoints tried after all primary endpoints have failed.
    pub fallback_endpoints: Vec<String>,
    /// Optional `x-token` auth header sent with every request.
    pub x_token: Option<String>,
    /// Timeout for establishing the gRPC connection.
    pub connect_timeout: Duration,
    /// Timeout applied to the underlying HTTP/2 request.
    pub request_timeout: Duration,
    /// Capacity of the bounded channel updates are delivered through.
    ///
    /// This is the adapter's backpressure knob: once full, the ingestion
    /// loop's `send` awaits until the consumer drains it, naturally slowing
    /// the subscription rather than dropping or unboundedly buffering data.
    pub subscription_buffer: usize,
    /// Initial backoff before the first reconnect attempt.
    pub initial_backoff: Duration,
    /// Upper bound on reconnect backoff.
    pub max_backoff: Duration,
    /// Backoff growth factor between reconnect attempts.
    pub backoff_multiplier: f64,
    /// If no update (including keepalive pings) is received within this
    /// window, the connection is considered unhealthy and torn down.
    pub health_check_interval: Duration,
    pub stale_after: Duration,
}

impl YellowstoneConfig {
    /// Create a configuration with a single primary endpoint and sensible defaults.
    pub fn new(endpoint: impl Into<String>) -> Self {
        YellowstoneConfig {
            primary_endpoints: vec![endpoint.into()],
            fallback_endpoints: Vec::new(),
            x_token: None,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            subscription_buffer: 10_000,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            health_check_interval: Duration::from_secs(1),
            stale_after: Duration::from_secs(5),
        }
    }

    /// Add fallback endpoints tried after the primary endpoints are exhausted.
    pub fn with_fallback_endpoints(mut self, endpoints: Vec<String>) -> Self {
        self.fallback_endpoints = endpoints;
        self
    }

    /// Set the `x-token` authentication header.
    pub fn with_x_token(mut self, token: impl Into<String>) -> Self {
        self.x_token = Some(token.into());
        self
    }

    /// Set the bounded delivery channel capacity.
    pub fn with_subscription_buffer(mut self, buffer: usize) -> Self {
        self.subscription_buffer = buffer;
        self
    }

    /// All endpoints in connection attempt order: primary, then fallback.
    pub fn endpoint_pool(&self) -> Vec<String> {
        self.primary_endpoints
            .iter()
            .chain(self.fallback_endpoints.iter())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = YellowstoneConfig::new("https://example.com:10900");
        assert_eq!(config.primary_endpoints.len(), 1);
        assert!(config.fallback_endpoints.is_empty());
        assert_eq!(config.subscription_buffer, 10_000);
    }

    #[test]
    fn test_endpoint_pool_order() {
        let config = YellowstoneConfig::new("primary")
            .with_fallback_endpoints(vec!["fallback1".to_string(), "fallback2".to_string()]);

        assert_eq!(
            config.endpoint_pool(),
            vec![
                "primary".to_string(),
                "fallback1".to_string(),
                "fallback2".to_string()
            ]
        );
    }

    #[test]
    fn test_builder_methods() {
        let config = YellowstoneConfig::new("primary")
            .with_x_token("secret")
            .with_subscription_buffer(500);

        assert_eq!(config.x_token, Some("secret".to_string()));
        assert_eq!(config.subscription_buffer, 500);
    }
}
