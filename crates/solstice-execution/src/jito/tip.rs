//! Tip construction. Jito prioritizes bundles by tip size, paid as a plain
//! SOL transfer to one of the Block Engine's published tip accounts
//! (fetched live via [`super::client::JitoClient::get_tip_accounts`], never
//! hardcoded here — that list can change, and paying a stale or wrong
//! account burns real SOL for nothing).

use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
// `solana_sdk::system_instruction` is deprecated in favor of the standalone
// `solana-system-interface` crate, but solana-sdk 2.x still ships it and
// nothing else in this workspace has migrated yet — pulling in a second
// crate for one function isn't worth it here.
#[allow(deprecated)]
use solana_sdk::system_instruction;

/// How the tip amount for a bundle is chosen, denominated in lamports so
/// this module never needs a SOL/USD conversion of its own.
#[derive(Debug, Clone, Copy)]
pub enum TipStrategy {
    /// A fixed tip, in lamports, regardless of bundle content.
    Fixed(u64),
    /// A tip proportional to the bundle's notional value (also in
    /// lamports), clamped to `[min_lamports, max_lamports]`.
    BpsOfNotional {
        bps: f64,
        min_lamports: u64,
        max_lamports: u64,
    },
}

impl TipStrategy {
    pub fn lamports_for(&self, notional_lamports: u64) -> u64 {
        match *self {
            TipStrategy::Fixed(lamports) => lamports,
            TipStrategy::BpsOfNotional {
                bps,
                min_lamports,
                max_lamports,
            } => {
                let scaled = (notional_lamports as f64 * (bps.max(0.0) / 10_000.0)) as u64;
                scaled.clamp(min_lamports, max_lamports)
            }
        }
    }
}

/// Build the tip instruction: a plain system-program transfer from `payer`
/// to `tip_account`, for `lamports`. Deliberately the simplest possible
/// instruction — Jito's tip mechanism needs nothing more than a SOL
/// transfer landing in the bundle alongside the real transactions.
pub fn build_tip_instruction(payer: &Pubkey, tip_account: &Pubkey, lamports: u64) -> Instruction {
    system_instruction::transfer(payer, tip_account, lamports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_tip() {
        let strategy = TipStrategy::Fixed(10_000);
        assert_eq!(strategy.lamports_for(1_000_000_000), 10_000);
        assert_eq!(strategy.lamports_for(0), 10_000);
    }

    #[test]
    fn test_bps_of_notional_scales() {
        let strategy = TipStrategy::BpsOfNotional {
            bps: 10.0, // 0.1%
            min_lamports: 1_000,
            max_lamports: 1_000_000,
        };
        // 1 SOL notional (1e9 lamports) * 0.1% = 1e6 lamports.
        assert_eq!(strategy.lamports_for(1_000_000_000), 1_000_000);
    }

    #[test]
    fn test_bps_of_notional_respects_floor() {
        let strategy = TipStrategy::BpsOfNotional {
            bps: 10.0,
            min_lamports: 5_000,
            max_lamports: 1_000_000,
        };
        assert_eq!(strategy.lamports_for(1_000), 5_000);
    }

    #[test]
    fn test_bps_of_notional_respects_ceiling() {
        let strategy = TipStrategy::BpsOfNotional {
            bps: 500.0, // 5%, deliberately huge to hit the cap
            min_lamports: 1_000,
            max_lamports: 50_000,
        };
        assert_eq!(strategy.lamports_for(1_000_000_000), 50_000);
    }

    #[test]
    fn test_tip_instruction_is_a_system_transfer() {
        let payer = Pubkey::new_unique();
        let tip_account = Pubkey::new_unique();
        let ix = build_tip_instruction(&payer, &tip_account, 12_345);

        assert_eq!(ix.program_id, solana_sdk::system_program::id());
        assert_eq!(ix.accounts[0].pubkey, payer);
        assert!(ix.accounts[0].is_signer);
        assert_eq!(ix.accounts[1].pubkey, tip_account);
    }
}
