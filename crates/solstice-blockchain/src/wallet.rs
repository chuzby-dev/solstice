//! Local wallet file management: generating and loading a keypair from a
//! JSON file on disk (the standard Solana CLI keypair format — a plain
//! array of 64 bytes — so it's interoperable with `solana-keygen` and
//! hardware/other wallet exports).
//!
//! This module never transmits a private key over the network and never
//! logs one. [`WalletFile::pubkey`] is the accessor every read-only
//! caller (a balance check, a deposit-address display) should use;
//! [`WalletFile::load_keypair`] — which actually materializes the private
//! key in memory — exists only for the moment code needs to sign a
//! transaction, and callers should not log, print, or persist its result
//! anywhere beyond that immediate use.

use crate::error::{BlockchainError, BlockchainResult};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signer};
use std::path::{Path, PathBuf};

pub struct WalletFile {
    path: PathBuf,
}

impl WalletFile {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        WalletFile { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Generate a new keypair and write it to this file. Refuses to
    /// overwrite an existing file — the whole point of a wallet file is
    /// that it might hold real funds, so silently clobbering one on a
    /// second call would be a real way to lose them.
    pub fn generate(&self) -> BlockchainResult<Pubkey> {
        if self.path.exists() {
            return Err(BlockchainError::TransactionError(format!(
                "refusing to overwrite existing wallet file at {}",
                self.path.display()
            )));
        }

        let keypair = Keypair::new();
        let bytes: Vec<u8> = keypair.to_bytes().to_vec();
        let json = serde_json::to_string(&bytes)
            .map_err(|e| BlockchainError::SerializationError(e.to_string()))?;

        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| BlockchainError::TransactionError(e.to_string()))?;
            }
        }
        std::fs::write(&self.path, json)
            .map_err(|e| BlockchainError::TransactionError(e.to_string()))?;

        Ok(keypair.pubkey())
    }

    fn load_keypair_bytes(&self) -> BlockchainResult<Vec<u8>> {
        let contents = std::fs::read_to_string(&self.path).map_err(|e| {
            BlockchainError::TransactionError(format!(
                "failed to read wallet file {}: {e}",
                self.path.display()
            ))
        })?;
        serde_json::from_str(&contents).map_err(|e| {
            BlockchainError::SerializationError(format!(
                "wallet file {} is not a valid keypair JSON array: {e}",
                self.path.display()
            ))
        })
    }

    /// The wallet's public address — safe to display, log, or send to a
    /// client; this is not sensitive.
    pub fn pubkey(&self) -> BlockchainResult<Pubkey> {
        Ok(self.load_keypair()?.pubkey())
    }

    /// The full signing keypair. Only call this at the point code is about
    /// to sign a transaction; never log or persist what this returns.
    pub fn load_keypair(&self) -> BlockchainResult<Keypair> {
        let bytes = self.load_keypair_bytes()?;
        Keypair::try_from(bytes.as_slice())
            .map_err(|e| BlockchainError::InvalidSignature(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signer::Signer;

    fn temp_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "solstice-wallet-test-{name}-{:?}-{nanos}.json",
            std::thread::current().id()
        ))
    }

    #[test]
    fn test_generate_and_load_roundtrip() {
        let path = temp_path("roundtrip");
        let wallet = WalletFile::at(&path);

        let generated_pubkey = wallet.generate().unwrap();
        let loaded_pubkey = wallet.pubkey().unwrap();
        assert_eq!(generated_pubkey, loaded_pubkey);

        let keypair = wallet.load_keypair().unwrap();
        assert_eq!(keypair.pubkey(), generated_pubkey);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_generate_refuses_to_overwrite() {
        let path = temp_path("no-overwrite");
        let wallet = WalletFile::at(&path);

        let first_pubkey = wallet.generate().unwrap();
        let result = wallet.generate();
        assert!(result.is_err());

        // The original wallet must be untouched.
        assert_eq!(wallet.pubkey().unwrap(), first_pubkey);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_exists_reflects_file_state() {
        let path = temp_path("exists");
        let wallet = WalletFile::at(&path);
        assert!(!wallet.exists());

        wallet.generate().unwrap();
        assert!(wallet.exists());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_keypair_missing_file_errors_cleanly() {
        let wallet = WalletFile::at(temp_path("missing"));
        assert!(wallet.load_keypair().is_err());
    }

    #[test]
    fn test_load_keypair_invalid_json_errors_cleanly() {
        let path = temp_path("invalid");
        std::fs::write(&path, "not valid json").unwrap();
        let wallet = WalletFile::at(&path);

        assert!(wallet.load_keypair().is_err());
        std::fs::remove_file(&path).ok();
    }
}
