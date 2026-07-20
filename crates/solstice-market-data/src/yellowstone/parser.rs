//! Parsing of Yellowstone account updates into [`MarketEvent`]s.

use crate::error::{MarketDataError, MarketDataResult};
use chrono::Utc;
use solana_sdk::pubkey::Pubkey;
use solstice_core::types::MarketEvent;
use yellowstone_grpc_proto::geyser::SubscribeUpdateAccount;

/// Parses raw Yellowstone account updates into core market events.
pub struct YellowstoneParser;

impl YellowstoneParser {
    /// Parse a single account update.
    ///
    /// Returns an error if the update is missing its account payload or if
    /// the pubkey/owner byte slices are not valid 32-byte Solana addresses
    /// (both would indicate a protocol-level problem, not a filterable
    /// application condition).
    pub fn parse_account_update(update: &SubscribeUpdateAccount) -> MarketDataResult<MarketEvent> {
        let account = update.account.as_ref().ok_or_else(|| {
            MarketDataError::InvalidData("account update missing account payload".to_string())
        })?;

        let address = Pubkey::try_from(account.pubkey.as_slice()).map_err(|_| {
            MarketDataError::InvalidData("account update has malformed pubkey".to_string())
        })?;
        let owner = Pubkey::try_from(account.owner.as_slice()).map_err(|_| {
            MarketDataError::InvalidData("account update has malformed owner".to_string())
        })?;

        Ok(MarketEvent::AccountUpdate {
            address,
            owner,
            lamports: account.lamports,
            data: account.data.clone(),
            slot: update.slot,
            timestamp: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yellowstone_grpc_proto::geyser::SubscribeUpdateAccountInfo;

    fn sample_update(pubkey: Vec<u8>, owner: Vec<u8>) -> SubscribeUpdateAccount {
        SubscribeUpdateAccount {
            account: Some(SubscribeUpdateAccountInfo {
                pubkey,
                lamports: 1_000_000,
                owner,
                executable: false,
                rent_epoch: 0,
                data: vec![1, 2, 3],
                write_version: 1,
                txn_signature: None,
            }),
            slot: 12345,
            is_startup: false,
        }
    }

    #[test]
    fn test_parse_valid_update() {
        let address = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let update = sample_update(address.to_bytes().to_vec(), owner.to_bytes().to_vec());

        let event = YellowstoneParser::parse_account_update(&update).unwrap();
        match event {
            MarketEvent::AccountUpdate {
                address: parsed_address,
                owner: parsed_owner,
                lamports,
                data,
                slot,
                ..
            } => {
                assert_eq!(parsed_address, address);
                assert_eq!(parsed_owner, owner);
                assert_eq!(lamports, 1_000_000);
                assert_eq!(data, vec![1, 2, 3]);
                assert_eq!(slot, 12345);
            }
            _ => panic!("expected AccountUpdate"),
        }
    }

    #[test]
    fn test_parse_missing_account() {
        let update = SubscribeUpdateAccount {
            account: None,
            slot: 1,
            is_startup: false,
        };

        assert!(YellowstoneParser::parse_account_update(&update).is_err());
    }

    #[test]
    fn test_parse_malformed_pubkey() {
        let update = sample_update(vec![1, 2, 3], Pubkey::new_unique().to_bytes().to_vec());

        assert!(YellowstoneParser::parse_account_update(&update).is_err());
    }
}
