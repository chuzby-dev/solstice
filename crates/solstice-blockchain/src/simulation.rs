//! Transaction simulation and validation.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Transaction simulation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    /// Whether simulation was successful.
    pub success: bool,

    /// Logs produced during simulation.
    pub logs: Vec<String>,

    /// Compute units consumed.
    pub compute_units_consumed: Option<u64>,

    /// Error message if simulation failed.
    pub error: Option<String>,

    /// Return data from the transaction (if any).
    pub return_data: Option<Vec<u8>>,

    /// Pre-execution account state changes (for debugging).
    pub pre_balances: Option<Vec<u64>>,

    /// Post-execution account state changes (for debugging).
    pub post_balances: Option<Vec<u64>>,

    /// Inner instructions (if any).
    pub inner_instructions: Option<Vec<InnerInstruction>>,
}

impl SimulationResult {
    /// Create a successful simulation result.
    pub fn success(logs: Vec<String>, compute_units_consumed: u64) -> Self {
        SimulationResult {
            success: true,
            logs,
            compute_units_consumed: Some(compute_units_consumed),
            error: None,
            return_data: None,
            pre_balances: None,
            post_balances: None,
            inner_instructions: None,
        }
    }

    /// Create a failed simulation result.
    pub fn failure(error: String, logs: Vec<String>) -> Self {
        SimulationResult {
            success: false,
            logs,
            compute_units_consumed: None,
            error: Some(error),
            return_data: None,
            pre_balances: None,
            post_balances: None,
            inner_instructions: None,
        }
    }

    /// Check if there was an error.
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Get error message if present.
    pub fn error_message(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Estimate cost based on compute units (in lamports).
    pub fn estimate_cost_lamports(&self) -> u64 {
        const LAMPORTS_PER_COMPUTE_UNIT: u64 = 1; // Rough estimate

        self.compute_units_consumed
            .unwrap_or(0)
            .saturating_mul(LAMPORTS_PER_COMPUTE_UNIT)
    }
}

impl fmt::Display for SimulationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SimulationResult {{ success: {}, compute_units: {:?}, error: {:?} }}",
            self.success, self.compute_units_consumed, self.error
        )
    }
}

/// Inner instruction from a program invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerInstruction {
    pub index: usize,
    pub instructions: Vec<String>, // Simplified representation
}

/// Configuration for transaction simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    /// Simulate transaction with commitment level.
    pub commitment: String,

    /// Include account state changes in results.
    pub include_state_changes: bool,

    /// Include return data if available.
    pub include_return_data: bool,

    /// Timeout for simulation in seconds.
    pub timeout_secs: u64,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        SimulationConfig {
            commitment: "confirmed".to_string(),
            include_state_changes: false,
            include_return_data: true,
            timeout_secs: 30,
        }
    }
}

/// Simulation error types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SimulationErrorKind {
    /// Invalid instruction in the transaction.
    InvalidInstruction(String),

    /// Account not found.
    AccountNotFound(String),

    /// Insufficient balance.
    InsufficientBalance,

    /// Program error.
    ProgramError(String),

    /// Instruction error.
    InstructionError { index: usize, message: String },

    /// Transaction error.
    TransactionError(String),

    /// Unknown error.
    Unknown(String),
}

impl fmt::Display for SimulationErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInstruction(msg) => write!(f, "Invalid instruction: {}", msg),
            Self::AccountNotFound(acct) => write!(f, "Account not found: {}", acct),
            Self::InsufficientBalance => write!(f, "Insufficient balance"),
            Self::ProgramError(msg) => write!(f, "Program error: {}", msg),
            Self::InstructionError { index, message } => {
                write!(f, "Instruction error at index {}: {}", index, message)
            }
            Self::TransactionError(msg) => write!(f, "Transaction error: {}", msg),
            Self::Unknown(msg) => write!(f, "Unknown error: {}", msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulation_success() {
        let result = SimulationResult::success(vec!["Log 1".to_string()], 100);
        assert!(result.success);
        assert_eq!(result.compute_units_consumed, Some(100));
        assert!(!result.has_error());
    }

    #[test]
    fn test_simulation_failure() {
        let result =
            SimulationResult::failure("Program failed".to_string(), vec!["Error log".to_string()]);
        assert!(!result.success);
        assert!(result.has_error());
        assert_eq!(result.error_message(), Some("Program failed"));
    }

    #[test]
    fn test_cost_estimation() {
        let result = SimulationResult::success(vec![], 50000);
        let cost = result.estimate_cost_lamports();
        assert_eq!(cost, 50000);
    }

    #[test]
    fn test_simulation_config_default() {
        let config = SimulationConfig::default();
        assert_eq!(config.commitment, "confirmed");
        assert_eq!(config.timeout_secs, 30);
    }

    #[test]
    fn test_error_kind_display() {
        let error = SimulationErrorKind::InsufficientBalance;
        assert!(error.to_string().contains("Insufficient balance"));
    }

    #[test]
    fn test_simulation_result_serialization() {
        let result = SimulationResult::success(vec!["test log".to_string()], 1000);
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: SimulationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.success, result.success);
    }
}
