//! Solstice Blockchain Integration
//!
//! This crate provides abstractions for interacting with the Solana blockchain,
//! including RPC client management, connection pooling, and failover strategies.

pub mod client;
pub mod error;
pub mod types;

pub use client::SolanaRpcClient;
pub use error::{BlockchainResult, BlockchainError};
pub use types::*;

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::SolanaRpcClient>;
    }
}
