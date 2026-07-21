//! Jito Block Engine client: bundle submission across one or more
//! endpoints, tip-account discovery, and bundle-status polling.
//!
//! Request/response shapes here follow Jito's published JSON-RPC API
//! (`getTipAccounts`, `sendBundle`, `getBundleStatuses`). `getTipAccounts`
//! was verified live against `https://mainnet.block-engine.jito.wtf` while
//! building this — see the Phase 5 changelog entry. `sendBundle` and
//! `getBundleStatuses` were not exercised against a real submission: doing
//! so needs a real signed transaction and real SOL for the tip, which this
//! agent does not hold and will not acquire. Their parsing is unit-tested
//! against the documented response shape instead.

use super::bundle::{Bundle, BundleStatus};
use super::error::{JitoError, JitoResult};
use base64::Engine as _;
use serde_json::{json, Value};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::Transaction;
use std::str::FromStr;
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct JitoConfig {
    /// Block Engine JSON-RPC endpoints to submit bundles to. Configuring
    /// more than one (e.g. multiple regions) is the "bundle redundancy"
    /// Phase 5.2 asks for — the identical signed bundle is offered to each
    /// in turn until one accepts it.
    pub endpoints: Vec<String>,
    pub http_timeout: Duration,
}

impl Default for JitoConfig {
    fn default() -> Self {
        JitoConfig {
            endpoints: vec!["https://mainnet.block-engine.jito.wtf/api/v1/bundles".to_string()],
            http_timeout: Duration::from_secs(10),
        }
    }
}

pub struct JitoClient {
    config: JitoConfig,
    http: reqwest::Client,
}

impl JitoClient {
    pub fn new(config: JitoConfig) -> JitoResult<Self> {
        if config.endpoints.is_empty() {
            return Err(JitoError::InvalidResponse(
                "no Block Engine endpoints configured".to_string(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(config.http_timeout)
            .build()
            .map_err(|e| JitoError::Http(e.to_string()))?;
        Ok(JitoClient { config, http })
    }

    /// Fetch the Block Engine's current tip accounts. Queried live rather
    /// than hardcoded: paying a tip to a stale account is real SOL spent
    /// for nothing.
    pub async fn get_tip_accounts(&self) -> JitoResult<Vec<Pubkey>> {
        let endpoint = self
            .config
            .endpoints
            .first()
            .expect("checked non-empty in new()");
        let body = build_rpc_request("getTipAccounts", json!([]));
        let response = self.post(endpoint, &body).await?;
        parse_tip_accounts_response(&response)
    }

    /// Submit `bundle` to each configured endpoint in turn, returning the
    /// bundle id from the first one that accepts it. Fails only if every
    /// endpoint rejects it.
    pub async fn send_bundle(&self, bundle: &Bundle) -> JitoResult<String> {
        bundle.validate()?;
        let body = build_send_bundle_request(bundle.transactions())?;

        let mut last_error = String::new();
        for endpoint in &self.config.endpoints {
            match self
                .post(endpoint, &body)
                .await
                .and_then(|r| parse_send_bundle_response(&r))
            {
                Ok(bundle_id) => return Ok(bundle_id),
                Err(e) => {
                    warn!("Jito endpoint {} rejected bundle: {}", endpoint, e);
                    last_error = e.to_string();
                }
            }
        }

        Err(JitoError::AllEndpointsFailed(last_error))
    }

    /// Query the landing status of a previously submitted bundle.
    pub async fn get_bundle_status(&self, bundle_id: &str) -> JitoResult<BundleStatus> {
        let endpoint = self
            .config
            .endpoints
            .first()
            .expect("checked non-empty in new()");
        let body = build_rpc_request("getBundleStatuses", json!([[bundle_id]]));
        let response = self.post(endpoint, &body).await?;
        parse_bundle_status_response(&response, bundle_id)
    }

    /// Poll [`Self::get_bundle_status`] until it lands, fails, or `timeout`
    /// elapses.
    pub async fn confirm_bundle(
        &self,
        bundle_id: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> JitoResult<BundleStatus> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.get_bundle_status(bundle_id).await? {
                BundleStatus::Pending => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(JitoError::ConfirmationTimeout);
                    }
                    tokio::time::sleep(poll_interval).await;
                }
                other => return Ok(other),
            }
        }
    }

    async fn post(&self, endpoint: &str, body: &Value) -> JitoResult<Value> {
        let response = self
            .http
            .post(endpoint)
            .json(body)
            .send()
            .await
            .map_err(|e| JitoError::Http(e.to_string()))?;
        response
            .json::<Value>()
            .await
            .map_err(|e| JitoError::Http(e.to_string()))
    }
}

fn build_rpc_request(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

fn encode_transaction(transaction: &Transaction) -> JitoResult<String> {
    let bytes =
        bincode::serialize(transaction).map_err(|e| JitoError::Serialization(e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn build_send_bundle_request(transactions: &[Transaction]) -> JitoResult<Value> {
    let encoded = transactions
        .iter()
        .map(encode_transaction)
        .collect::<JitoResult<Vec<String>>>()?;
    Ok(build_rpc_request(
        "sendBundle",
        json!([encoded, { "encoding": "base64" }]),
    ))
}

fn parse_rpc_error(value: &Value) -> Option<JitoError> {
    let err = value.get("error")?;
    Some(JitoError::Rpc {
        code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
        message: err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string(),
    })
}

fn parse_send_bundle_response(value: &Value) -> JitoResult<String> {
    if let Some(err) = parse_rpc_error(value) {
        return Err(err);
    }
    value
        .get("result")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| JitoError::InvalidResponse(value.to_string()))
}

fn parse_tip_accounts_response(value: &Value) -> JitoResult<Vec<Pubkey>> {
    if let Some(err) = parse_rpc_error(value) {
        return Err(err);
    }
    let entries = value
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| JitoError::InvalidResponse(value.to_string()))?;

    entries
        .iter()
        .map(|entry| {
            let s = entry.as_str().ok_or_else(|| {
                JitoError::InvalidResponse("tip account not a string".to_string())
            })?;
            Pubkey::from_str(s).map_err(|e| {
                JitoError::InvalidResponse(format!("invalid tip account pubkey {s}: {e}"))
            })
        })
        .collect()
}

fn parse_bundle_status_response(value: &Value, bundle_id: &str) -> JitoResult<BundleStatus> {
    if let Some(err) = parse_rpc_error(value) {
        return Err(err);
    }
    let result = value
        .get("result")
        .ok_or_else(|| JitoError::InvalidResponse(value.to_string()))?;
    let statuses = result
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| JitoError::InvalidResponse(value.to_string()))?;

    let Some(entry) = statuses
        .iter()
        .find(|e| e.get("bundle_id").and_then(Value::as_str) == Some(bundle_id))
    else {
        return Ok(BundleStatus::Pending);
    };

    if let Some(err_value) = entry.get("err") {
        if !err_value.is_null() {
            return Ok(BundleStatus::Failed {
                reason: err_value.to_string(),
            });
        }
    }

    let slot = entry.get("slot").and_then(Value::as_u64).unwrap_or(0);
    let confirmation_status = entry
        .get("confirmation_status")
        .and_then(Value::as_str)
        .unwrap_or("processed")
        .to_string();
    Ok(BundleStatus::Landed {
        slot,
        confirmation_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::message::Message;

    fn dummy_transaction() -> Transaction {
        Transaction::new_unsigned(Message::default())
    }

    #[test]
    fn test_encode_transaction_produces_valid_base64() {
        let encoded = encode_transaction(&dummy_transaction()).unwrap();
        assert!(base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .is_ok());
    }

    #[test]
    fn test_build_send_bundle_request_shape() {
        let request = build_send_bundle_request(&[dummy_transaction()]).unwrap();
        assert_eq!(request["method"], "sendBundle");
        assert_eq!(request["params"][1]["encoding"], "base64");
        assert_eq!(request["params"][0].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_parse_send_bundle_response_success() {
        let response = json!({ "jsonrpc": "2.0", "id": 1, "result": "abc123" });
        assert_eq!(
            parse_send_bundle_response(&response).unwrap(),
            "abc123".to_string()
        );
    }

    #[test]
    fn test_parse_send_bundle_response_rpc_error() {
        let response = json!({
            "jsonrpc": "2.0", "id": 1,
            "error": { "code": -32000, "message": "bundle rejected" }
        });
        let result = parse_send_bundle_response(&response);
        assert!(matches!(result, Err(JitoError::Rpc { code: -32000, .. })));
    }

    #[test]
    fn test_parse_tip_accounts_response() {
        // Shape verified live against https://mainnet.block-engine.jito.wtf.
        let response = json!({
            "jsonrpc": "2.0",
            "result": [
                "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
                "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5"
            ],
            "id": 1
        });
        let accounts = parse_tip_accounts_response(&response).unwrap();
        assert_eq!(accounts.len(), 2);
    }

    #[test]
    fn test_parse_tip_accounts_response_rejects_invalid_pubkey() {
        let response = json!({ "jsonrpc": "2.0", "result": ["not-a-pubkey"], "id": 1 });
        assert!(matches!(
            parse_tip_accounts_response(&response),
            Err(JitoError::InvalidResponse(_))
        ));
    }

    #[test]
    fn test_parse_bundle_status_pending_when_not_found() {
        let response = json!({
            "jsonrpc": "2.0",
            "result": { "context": { "slot": 100 }, "value": [] },
            "id": 1
        });
        assert_eq!(
            parse_bundle_status_response(&response, "bundle-1").unwrap(),
            BundleStatus::Pending
        );
    }

    #[test]
    fn test_parse_bundle_status_landed() {
        let response = json!({
            "jsonrpc": "2.0",
            "result": {
                "context": { "slot": 100 },
                "value": [{
                    "bundle_id": "bundle-1",
                    "transactions": ["sig1"],
                    "slot": 12345,
                    "confirmation_status": "confirmed",
                    "err": null
                }]
            },
            "id": 1
        });
        let status = parse_bundle_status_response(&response, "bundle-1").unwrap();
        assert_eq!(
            status,
            BundleStatus::Landed {
                slot: 12345,
                confirmation_status: "confirmed".to_string()
            }
        );
    }

    #[tokio::test]
    #[ignore = "requires a live, reachable Jito Block Engine endpoint"]
    async fn test_get_tip_accounts_live() {
        let client = JitoClient::new(JitoConfig::default()).unwrap();
        let accounts = client.get_tip_accounts().await.unwrap();
        assert!(!accounts.is_empty());
    }

    #[test]
    fn test_parse_bundle_status_failed() {
        let response = json!({
            "jsonrpc": "2.0",
            "result": {
                "context": { "slot": 100 },
                "value": [{
                    "bundle_id": "bundle-1",
                    "transactions": [],
                    "slot": 0,
                    "confirmation_status": null,
                    "err": { "Ok": null }
                }]
            },
            "id": 1
        });
        let status = parse_bundle_status_response(&response, "bundle-1").unwrap();
        assert!(matches!(status, BundleStatus::Failed { .. }));
    }
}
