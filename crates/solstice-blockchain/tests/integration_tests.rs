//! Integration tests for blockchain module.

use solana_sdk::pubkey::Pubkey;
use solstice_blockchain::{
    AccountInfo, RpcClientConfig, RpcEndpointConfig, SimulationResult, SolanaRpcClient,
    TransactionBuilder,
};

#[test]
fn test_rpc_client_creation_with_endpoints() {
    let config = RpcClientConfig {
        endpoints: vec![
            RpcEndpointConfig::new("http://localhost:8899".to_string()),
            RpcEndpointConfig::new("http://backup:8899".to_string()),
        ],
        ..Default::default()
    };

    let client = SolanaRpcClient::new(config).expect("Failed to create RPC client");
    assert!(client.get_active_endpoint().is_ok());
}

#[test]
fn test_rpc_client_with_helper() {
    let result = SolanaRpcClient::with_endpoints(vec!["http://localhost:8899".to_string()]);

    assert!(result.is_ok());
}

#[test]
fn test_transaction_builder_basic() {
    let builder = TransactionBuilder::new();
    assert!(builder.is_empty());
    assert_eq!(builder.instruction_count(), 0);

    let builder = builder.payer(Pubkey::new_unique());
    assert_eq!(builder.instruction_count(), 0);
}

#[test]
fn test_transaction_builder_size_estimation() {
    let builder = TransactionBuilder::new().payer(Pubkey::new_unique());
    let size = builder.estimate_size();

    // Should have at least header size
    assert!(size > 0);
}

#[test]
fn test_simulation_success_result() {
    let result = SimulationResult::success(vec!["Log 1".to_string(), "Log 2".to_string()], 50000);

    assert!(result.success);
    assert!(!result.has_error());
    assert_eq!(result.logs.len(), 2);
    assert_eq!(result.compute_units_consumed, Some(50000));
}

#[test]
fn test_simulation_failure_result() {
    let result = SimulationResult::failure(
        "Program error: insufficient funds".to_string(),
        vec!["Error log".to_string()],
    );

    assert!(!result.success);
    assert!(result.has_error());
    assert_eq!(
        result.error_message(),
        Some("Program error: insufficient funds")
    );
}

#[test]
fn test_simulation_cost_estimation() {
    let result = SimulationResult::success(vec![], 100000);
    let cost = result.estimate_cost_lamports();

    // With base rate of 1 lamport per compute unit
    assert_eq!(cost, 100000);
}

#[test]
fn test_account_info_creation() {
    let address = Pubkey::new_unique();
    let owner = Pubkey::new_unique();

    let info = AccountInfo::new_without_data(address, owner, 1000000, false);

    assert_eq!(info.address, address);
    assert_eq!(info.owner, owner);
    assert_eq!(info.lamports, 1000000);
    assert!(!info.executable);
    assert_eq!(info.data_size(), 0);
}

#[test]
fn test_account_info_with_data() {
    let mut info =
        AccountInfo::new_without_data(Pubkey::new_unique(), Pubkey::new_unique(), 1000000, false);

    let data = vec![0u8; 256];
    info.data = Some(data.clone());

    assert!(info.has_data());
    assert_eq!(info.data_size(), 256);
}

#[test]
fn test_account_ownership_check() {
    let owner = Pubkey::new_unique();
    let other = Pubkey::new_unique();

    let info = AccountInfo::new_without_data(Pubkey::new_unique(), owner, 1000, false);

    assert!(info.is_owned_by(&owner));
    assert!(!info.is_owned_by(&other));
}

#[test]
fn test_account_info_serialization() {
    let info =
        AccountInfo::new_without_data(Pubkey::new_unique(), Pubkey::new_unique(), 5000000, false);

    let json = serde_json::to_string(&info).expect("Serialization failed");
    let deserialized: AccountInfo = serde_json::from_str(&json).expect("Deserialization failed");

    assert_eq!(info.address, deserialized.address);
    assert_eq!(info.owner, deserialized.owner);
    assert_eq!(info.lamports, deserialized.lamports);
}

#[test]
fn test_simulation_result_serialization() {
    let result = SimulationResult::success(vec!["Test log".to_string()], 75000);

    let json = serde_json::to_string(&result).expect("Serialization failed");
    let deserialized: SimulationResult =
        serde_json::from_str(&json).expect("Deserialization failed");

    assert_eq!(result.success, deserialized.success);
    assert_eq!(
        result.compute_units_consumed,
        deserialized.compute_units_consumed
    );
}

#[test]
fn test_rpc_endpoint_health_tracking() {
    let config = RpcClientConfig {
        endpoints: vec![RpcEndpointConfig::new("http://localhost:8899".to_string())],
        ..Default::default()
    };

    let client = SolanaRpcClient::new(config).expect("Failed to create client");
    let endpoint = client.get_active_endpoint().expect("No active endpoint");

    // Record a success
    client
        .record_success(&endpoint, 50.0)
        .expect("Failed to record success");

    let health = client
        .get_active_endpoint_health()
        .expect("Failed to get health");
    assert!(health.is_healthy);
    assert_eq!(health.consecutive_errors, 0);

    // Record an error
    client
        .record_error(&endpoint, "Test error".to_string())
        .expect("Failed to record error");

    // Endpoint should still be healthy (1 error)
    let health = client
        .get_active_endpoint_health()
        .expect("Failed to get health");
    assert_eq!(health.consecutive_errors, 1);
}

#[test]
fn test_transaction_builder_default() {
    let builder = TransactionBuilder::default();
    assert!(builder.is_empty());
}

#[test]
fn test_simulation_config_default() {
    let config = solstice_blockchain::SimulationConfig::default();
    assert_eq!(config.commitment, "confirmed");
    assert_eq!(config.timeout_secs, 30);
    assert!(config.include_return_data);
}

#[test]
fn test_account_query_config_default() {
    let config = solstice_blockchain::AccountQueryConfig::default();
    assert!(config.include_data);
    assert_eq!(config.commitment, "confirmed");
}
