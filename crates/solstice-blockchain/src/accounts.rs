//! Account state queries and utilities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::{account::Account, pubkey::Pubkey};

/// Account information wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    /// Account public key.
    pub address: Pubkey,

    /// Account owner (program).
    pub owner: Pubkey,

    /// Account lamports balance.
    pub lamports: u64,

    /// Whether account is executable.
    pub executable: bool,

    /// Rent epoch.
    pub rent_epoch: u64,

    /// Account data (optional, may be large).
    pub data: Option<Vec<u8>>,

    /// Timestamp when account was queried.
    pub queried_at: DateTime<Utc>,
}

impl AccountInfo {
    /// Create account info from a Solana account.
    pub fn from_solana_account(address: Pubkey, account: Account) -> Self {
        AccountInfo {
            address,
            owner: account.owner,
            lamports: account.lamports,
            executable: account.executable,
            rent_epoch: account.rent_epoch,
            data: Some(account.data),
            queried_at: Utc::now(),
        }
    }

    /// Create account info without data.
    pub fn new_without_data(
        address: Pubkey,
        owner: Pubkey,
        lamports: u64,
        executable: bool,
    ) -> Self {
        AccountInfo {
            address,
            owner,
            lamports,
            executable,
            rent_epoch: 0,
            data: None,
            queried_at: Utc::now(),
        }
    }

    /// Check if account is owned by a specific program.
    pub fn is_owned_by(&self, program_id: &Pubkey) -> bool {
        self.owner == *program_id
    }

    /// Get account data size.
    pub fn data_size(&self) -> usize {
        self.data.as_ref().map(|d| d.len()).unwrap_or(0)
    }

    /// Check if account is a system account.
    pub fn is_system_account(&self) -> bool {
        self.owner == solana_sdk::system_program::ID
    }

    /// Check if account has data.
    pub fn has_data(&self) -> bool {
        self.data.is_some() && self.data_size() > 0
    }
}

/// Configuration for account queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountQueryConfig {
    /// Include account data in response.
    pub include_data: bool,

    /// Commitment level for queries.
    pub commitment: String,

    /// Timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for AccountQueryConfig {
    fn default() -> Self {
        AccountQueryConfig {
            include_data: true,
            commitment: "confirmed".to_string(),
            timeout_secs: 30,
        }
    }
}

/// Batch account query result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchAccountResult {
    /// Successfully retrieved accounts.
    pub accounts: Vec<AccountInfo>,

    /// Accounts that failed to retrieve with reasons.
    pub failed: Vec<(Pubkey, String)>,

    /// Total time taken in milliseconds.
    pub elapsed_ms: u64,
}

impl BatchAccountResult {
    /// Create a new batch result.
    pub fn new() -> Self {
        BatchAccountResult {
            accounts: Vec::new(),
            failed: Vec::new(),
            elapsed_ms: 0,
        }
    }

    /// Get success rate as percentage.
    pub fn success_rate(&self) -> f64 {
        let total = self.accounts.len() + self.failed.len();
        if total == 0 {
            0.0
        } else {
            (self.accounts.len() as f64 / total as f64) * 100.0
        }
    }

    /// Check if all queries succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.failed.is_empty()
    }
}

impl Default for BatchAccountResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Account program filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramFilter {
    /// Match specific program owner.
    Owner(Pubkey),

    /// Match any program (no filter).
    Any,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_info_creation() {
        let address = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let info = AccountInfo::new_without_data(address, owner, 1000, false);

        assert_eq!(info.address, address);
        assert_eq!(info.owner, owner);
        assert_eq!(info.lamports, 1000);
        assert!(!info.executable);
    }

    #[test]
    fn test_account_info_ownership() {
        let address = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let info = AccountInfo::new_without_data(address, owner, 1000, false);

        assert!(info.is_owned_by(&owner));
        assert!(!info.is_owned_by(&Pubkey::new_unique()));
    }

    #[test]
    fn test_account_info_data_size() {
        let mut info =
            AccountInfo::new_without_data(Pubkey::new_unique(), Pubkey::new_unique(), 1000, false);

        assert_eq!(info.data_size(), 0);

        info.data = Some(vec![0u8; 100]);
        assert_eq!(info.data_size(), 100);
    }

    #[test]
    fn test_batch_result_success_rate() {
        let mut result = BatchAccountResult::new();
        result.accounts.push(AccountInfo::new_without_data(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            1000,
            false,
        ));
        result.accounts.push(AccountInfo::new_without_data(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            2000,
            false,
        ));
        result
            .failed
            .push((Pubkey::new_unique(), "Not found".to_string()));

        assert!((result.success_rate() - 200.0 / 3.0).abs() < 1e-9); // ~66.67%
        assert!(!result.all_succeeded());
    }

    #[test]
    fn test_account_query_config_default() {
        let config = AccountQueryConfig::default();
        assert!(config.include_data);
        assert_eq!(config.commitment, "confirmed");
    }

    #[test]
    fn test_account_info_serialization() {
        let info =
            AccountInfo::new_without_data(Pubkey::new_unique(), Pubkey::new_unique(), 1000, false);

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: AccountInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(info.address, deserialized.address);
        assert_eq!(info.lamports, deserialized.lamports);
    }
}
