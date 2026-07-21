//! Solstice Blockchain Integration
//!
//! This crate provides abstractions for interacting with the Solana blockchain,
//! including RPC client management, connection pooling, failover strategies,
//! transaction building, and account queries.

pub mod accounts;
pub mod client;
pub mod error;
pub mod simulation;
pub mod transaction;
pub mod types;
pub mod wallet;

pub use accounts::{AccountInfo, AccountQueryConfig, BatchAccountResult};
pub use client::SolanaRpcClient;
pub use error::{BlockchainError, BlockchainResult};
pub use simulation::{SimulationConfig, SimulationErrorKind, SimulationResult};
pub use transaction::{SubmissionResult, TransactionBuilder};
pub use types::*;
pub use wallet::WalletFile;

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::SolanaRpcClient>;
    }
}
