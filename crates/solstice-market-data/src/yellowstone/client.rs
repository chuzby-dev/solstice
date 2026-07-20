//! Yellowstone gRPC client: connection management, subscription, automatic
//! reconnection with backoff, and health monitoring.

use crate::error::{MarketDataError, MarketDataResult};
use crate::yellowstone::config::YellowstoneConfig;
use crate::yellowstone::filter::AccountFilter;
use crate::yellowstone::parser::YellowstoneParser;
use solstice_core::types::MarketEvent;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;
use tracing::{debug, info, warn};
use yellowstone_grpc_proto::geyser::geyser_client::GeyserClient;
use yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof;
use yellowstone_grpc_proto::geyser::{CommitmentLevel, SubscribeRequest, SubscribeUpdatePong};

/// A running Yellowstone gRPC subscription.
///
/// Owns a background reconnect loop (started by [`spawn`](Self::spawn)) that
/// cycles through the configured endpoint pool with exponential backoff,
/// re-subscribing with the given filter after every reconnect, and forwards
/// parsed [`MarketEvent`]s over a bounded channel. The channel's capacity is
/// the adapter's backpressure mechanism: a full channel makes the ingestion
/// loop's `send` await rather than drop updates or grow without bound.
pub struct YellowstoneClient {
    config: YellowstoneConfig,
    last_update_unix_ms: Arc<AtomicI64>,
}

impl YellowstoneClient {
    pub fn new(config: YellowstoneConfig) -> Self {
        YellowstoneClient {
            config,
            last_update_unix_ms: Arc::new(AtomicI64::new(now_unix_ms())),
        }
    }

    /// Whether the connection has received an update (including keepalive
    /// pings) within `health_check_interval + stale_after` of now.
    pub fn is_healthy(&self) -> bool {
        let last = self.last_update_unix_ms.load(Ordering::Relaxed);
        let threshold_ms = self.config.stale_after.as_millis() as i64;
        now_unix_ms() - last <= threshold_ms
    }

    /// Milliseconds since the last received update.
    pub fn time_since_last_update_ms(&self) -> i64 {
        now_unix_ms() - self.last_update_unix_ms.load(Ordering::Relaxed)
    }

    /// Run the subscribe/reconnect loop until `sender` is dropped by the
    /// consumer, at which point this returns `Ok(())`.
    ///
    /// Connection and stream errors are logged and retried with exponential
    /// backoff across the endpoint pool; they never terminate the loop.
    pub async fn run(
        &self,
        filter: AccountFilter,
        commitment: CommitmentLevel,
        sender: mpsc::Sender<MarketEvent>,
    ) -> MarketDataResult<()> {
        if filter.is_empty() {
            return Err(MarketDataError::SubscriptionError(
                "account filter has no include/owner criteria".to_string(),
            ));
        }

        let endpoints = self.config.endpoint_pool();
        if endpoints.is_empty() {
            return Err(MarketDataError::ConnectionError(
                "no Yellowstone endpoints configured".to_string(),
            ));
        }

        let mut endpoint_idx = 0usize;
        let mut backoff = self.config.initial_backoff;

        loop {
            let endpoint = &endpoints[endpoint_idx % endpoints.len()];

            match self
                .run_single_connection(endpoint, &filter, commitment, &sender)
                .await
            {
                Ok(ConnectionOutcome::ConsumerClosed) => {
                    info!("Yellowstone consumer closed, stopping subscription");
                    return Ok(());
                }
                Ok(ConnectionOutcome::StreamEnded) => {
                    warn!("Yellowstone stream ended by server: {}", endpoint);
                    backoff = self.config.initial_backoff;
                }
                Err(e) => {
                    warn!("Yellowstone connection failed ({}): {}", endpoint, e);
                    endpoint_idx = endpoint_idx.wrapping_add(1);

                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(
                        Duration::from_secs_f64(
                            backoff.as_secs_f64() * self.config.backoff_multiplier,
                        ),
                        self.config.max_backoff,
                    );
                    continue;
                }
            }
        }
    }

    async fn run_single_connection(
        &self,
        endpoint: &str,
        filter: &AccountFilter,
        commitment: CommitmentLevel,
        sender: &mpsc::Sender<MarketEvent>,
    ) -> MarketDataResult<ConnectionOutcome> {
        let channel = self.connect(endpoint).await?;
        let mut client = GeyserClient::new(channel).max_decoding_message_size(64 * 1024 * 1024);

        let (outbound_tx, outbound_rx) = mpsc::channel::<SubscribeRequest>(16);
        outbound_tx
            .send(build_subscribe_request(filter, commitment))
            .await
            .map_err(|_| {
                MarketDataError::ConnectionError("failed to queue subscribe request".to_string())
            })?;

        let mut request = Request::new(ReceiverStream::new(outbound_rx));
        if let Some(token) = &self.config.x_token {
            let value = token.parse().map_err(|_| {
                MarketDataError::ConnectionError("invalid x-token header value".to_string())
            })?;
            request.metadata_mut().insert("x-token", value);
        }

        let response = client
            .subscribe(request)
            .await
            .map_err(|e| MarketDataError::ConnectionError(format!("subscribe failed: {e}")))?;
        let mut stream = response.into_inner();

        info!("Yellowstone connected: {}", endpoint);
        self.touch();

        loop {
            let message = match stream.message().await {
                Ok(Some(update)) => update,
                Ok(None) => return Ok(ConnectionOutcome::StreamEnded),
                Err(e) => {
                    return Err(MarketDataError::ConnectionError(format!(
                        "stream error: {e}"
                    )))
                }
            };
            self.touch();

            match message.update_oneof {
                Some(UpdateOneof::Account(account_update)) => {
                    match YellowstoneParser::parse_account_update(&account_update) {
                        Ok(event) => {
                            if !passes_filter(&event, filter) {
                                continue;
                            }
                            if sender.send(event).await.is_err() {
                                return Ok(ConnectionOutcome::ConsumerClosed);
                            }
                        }
                        Err(e) => {
                            warn!("Failed to parse Yellowstone account update: {}", e);
                        }
                    }
                }
                Some(UpdateOneof::Ping(_)) => {
                    debug!("Received Yellowstone ping, sending pong");
                    let pong = SubscribeRequest {
                        ping: Some(yellowstone_grpc_proto::geyser::SubscribeRequestPing { id: 1 }),
                        ..Default::default()
                    };
                    if outbound_tx.send(pong).await.is_err() {
                        return Ok(ConnectionOutcome::StreamEnded);
                    }
                }
                Some(UpdateOneof::Pong(SubscribeUpdatePong { id })) => {
                    debug!("Received Yellowstone pong (id={})", id);
                }
                _ => {}
            }
        }
    }

    async fn connect(&self, endpoint: &str) -> MarketDataResult<Channel> {
        let ep = Endpoint::from_shared(endpoint.to_string())
            .map_err(|e| MarketDataError::ConnectionError(format!("invalid endpoint: {e}")))?
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.request_timeout)
            .tls_config(tonic::transport::ClientTlsConfig::new().with_native_roots())
            .map_err(|e| MarketDataError::ConnectionError(format!("TLS config error: {e}")))?;

        ep.connect()
            .await
            .map_err(|e| MarketDataError::ConnectionError(format!("connect failed: {e}")))
    }

    fn touch(&self) {
        self.last_update_unix_ms
            .store(now_unix_ms(), Ordering::Relaxed);
    }
}

enum ConnectionOutcome {
    StreamEnded,
    ConsumerClosed,
}

fn build_subscribe_request(
    filter: &AccountFilter,
    commitment: CommitmentLevel,
) -> SubscribeRequest {
    let mut accounts = HashMap::new();
    accounts.insert("solstice".to_string(), filter.to_proto_filter());

    SubscribeRequest {
        accounts,
        commitment: Some(commitment as i32),
        ..Default::default()
    }
}

/// Re-check the parsed event against the filter's client-side-only criteria
/// (exclude list, min lamports) that the server-side filter cannot express.
fn passes_filter(event: &MarketEvent, filter: &AccountFilter) -> bool {
    match event {
        MarketEvent::AccountUpdate {
            address,
            owner,
            lamports,
            ..
        } => filter.should_subscribe(address, owner, *lamports),
        _ => true,
    }
}

fn now_unix_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn test_new_client_is_healthy_immediately() {
        let config = YellowstoneConfig::new("https://example.com:10900");
        let client = YellowstoneClient::new(config);
        assert!(client.is_healthy());
    }

    #[test]
    fn test_build_subscribe_request_contains_filter() {
        let program = Pubkey::new_unique();
        let filter = AccountFilter::new().owned_by(program);
        let request = build_subscribe_request(&filter, CommitmentLevel::Confirmed);

        assert!(request.accounts.contains_key("solstice"));
        assert_eq!(request.commitment, Some(CommitmentLevel::Confirmed as i32));
    }

    #[tokio::test]
    async fn test_run_rejects_empty_filter() {
        let config = YellowstoneConfig::new("https://example.com:10900");
        let client = YellowstoneClient::new(config);
        let (tx, _rx) = mpsc::channel(1);

        let result = client
            .run(AccountFilter::new(), CommitmentLevel::Confirmed, tx)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_rejects_no_endpoints() {
        let config = YellowstoneConfig {
            primary_endpoints: Vec::new(),
            ..YellowstoneConfig::new("unused")
        };
        let client = YellowstoneClient::new(config);
        let filter = AccountFilter::new().owned_by(Pubkey::new_unique());
        let (tx, _rx) = mpsc::channel(1);

        let result = client.run(filter, CommitmentLevel::Confirmed, tx).await;
        assert!(result.is_err());
    }
}
