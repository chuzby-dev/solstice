//! Account filtering: which accounts to subscribe to, and translation to the
//! Yellowstone wire filter format.

use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use yellowstone_grpc_proto::geyser::SubscribeRequestFilterAccounts;

/// Client-side account filter.
///
/// Mirrors the filter that is sent to the server (`to_proto_filter`), and is
/// also usable standalone (`should_subscribe`) to decide which accounts are
/// worth including in a subscription request in the first place, before any
/// bytes go over the wire.
#[derive(Debug, Clone, Default)]
pub struct AccountFilter {
    /// Always included, regardless of owner.
    include: HashSet<Pubkey>,
    /// Always excluded, even if owned by a program in `owner_programs`.
    exclude: HashSet<Pubkey>,
    /// Included if owned by any of these programs.
    owner_programs: Vec<Pubkey>,
    /// Minimum lamport balance required for inclusion (owner-program matches only).
    min_lamports: Option<u64>,
}

impl AccountFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn include_account(mut self, address: Pubkey) -> Self {
        self.include.insert(address);
        self
    }

    pub fn include_accounts(mut self, addresses: impl IntoIterator<Item = Pubkey>) -> Self {
        self.include.extend(addresses);
        self
    }

    pub fn exclude_account(mut self, address: Pubkey) -> Self {
        self.exclude.insert(address);
        self
    }

    pub fn owned_by(mut self, program_id: Pubkey) -> Self {
        self.owner_programs.push(program_id);
        self
    }

    pub fn min_lamports(mut self, lamports: u64) -> Self {
        self.min_lamports = Some(lamports);
        self
    }

    /// Whether an account with the given address/owner/lamports should be
    /// included in the subscription.
    pub fn should_subscribe(&self, address: &Pubkey, owner: &Pubkey, lamports: u64) -> bool {
        if self.exclude.contains(address) {
            return false;
        }
        if self.include.contains(address) {
            return true;
        }
        if !self.owner_programs.contains(owner) {
            return false;
        }
        match self.min_lamports {
            Some(min) => lamports >= min,
            None => true,
        }
    }

    /// Whether the filter has no positive criteria at all (would match nothing).
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.owner_programs.is_empty()
    }

    /// Translate to the wire format sent in `SubscribeRequest.accounts`.
    ///
    /// `exclude` and `min_lamports` have no equivalent server-side field in
    /// the Yellowstone protocol, so they remain client-side-only checks
    /// applied in [`should_subscribe`](Self::should_subscribe) and re-checked
    /// against every inbound update in the parser.
    pub fn to_proto_filter(&self) -> SubscribeRequestFilterAccounts {
        SubscribeRequestFilterAccounts {
            account: self.include.iter().map(|p| p.to_string()).collect(),
            owner: self.owner_programs.iter().map(|p| p.to_string()).collect(),
            filters: Vec::new(),
            nonempty_txn_signature: None,
            cuckoo_accounts_filter: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_subscribe_include() {
        let addr = Pubkey::new_unique();
        let filter = AccountFilter::new().include_account(addr);

        assert!(filter.should_subscribe(&addr, &Pubkey::new_unique(), 0));
    }

    #[test]
    fn test_should_subscribe_exclude_overrides_include() {
        let addr = Pubkey::new_unique();
        let filter = AccountFilter::new()
            .include_account(addr)
            .exclude_account(addr);

        assert!(!filter.should_subscribe(&addr, &Pubkey::new_unique(), 0));
    }

    #[test]
    fn test_should_subscribe_owner_program() {
        let program = Pubkey::new_unique();
        let addr = Pubkey::new_unique();
        let filter = AccountFilter::new().owned_by(program);

        assert!(filter.should_subscribe(&addr, &program, 0));
        assert!(!filter.should_subscribe(&addr, &Pubkey::new_unique(), 0));
    }

    #[test]
    fn test_should_subscribe_min_lamports() {
        let program = Pubkey::new_unique();
        let addr = Pubkey::new_unique();
        let filter = AccountFilter::new().owned_by(program).min_lamports(1_000);

        assert!(!filter.should_subscribe(&addr, &program, 999));
        assert!(filter.should_subscribe(&addr, &program, 1_000));
    }

    #[test]
    fn test_is_empty() {
        assert!(AccountFilter::new().is_empty());
        assert!(!AccountFilter::new()
            .include_account(Pubkey::new_unique())
            .is_empty());
    }

    #[test]
    fn test_to_proto_filter() {
        let program = Pubkey::new_unique();
        let account = Pubkey::new_unique();
        let filter = AccountFilter::new()
            .include_account(account)
            .owned_by(program);

        let proto = filter.to_proto_filter();
        assert_eq!(proto.account, vec![account.to_string()]);
        assert_eq!(proto.owner, vec![program.to_string()]);
    }
}
