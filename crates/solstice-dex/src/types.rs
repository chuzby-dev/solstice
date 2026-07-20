//! Shared DEX integration types: quotes, routes, and swap requests.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;

/// Request for a swap quote.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QuoteRequest {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub amount: u64,
    /// Maximum acceptable slippage, in basis points (50 = 0.5%).
    pub slippage_bps: u32,
}

impl QuoteRequest {
    pub fn new(input_mint: Pubkey, output_mint: Pubkey, amount: u64, slippage_bps: u32) -> Self {
        QuoteRequest {
            input_mint,
            output_mint,
            amount,
            slippage_bps,
        }
    }
}

/// One leg of a (possibly multi-hop) route.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteSegment {
    pub dex: String,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub input_amount: u64,
    pub output_amount: u64,
}

/// A quote for swapping `in_amount` of one token into `out_amount` of another.
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_bps: u32,
    /// Price impact as a decimal fraction (0.05 = 5%).
    pub price_impact: f64,
    /// Available liquidity at the quoted price, in the output token's base units.
    pub liquidity: u64,
    pub route: Vec<RouteSegment>,
    pub timestamp: DateTime<Utc>,
}

impl Quote {
    /// Minimum acceptable output after applying a slippage tolerance, in basis points.
    pub fn min_out_amount(&self, slippage_bps: u32) -> u64 {
        let bps = slippage_bps.min(10_000) as u128;
        let out = self.out_amount as u128;
        (out * (10_000 - bps) / 10_000) as u64
    }

    /// Effective price (output units per input unit).
    pub fn price(&self) -> f64 {
        if self.in_amount == 0 {
            0.0
        } else {
            self.out_amount as f64 / self.in_amount as f64
        }
    }
}

/// Request to build swap instructions from an already-obtained quote.
#[derive(Debug, Clone)]
pub struct SwapRequest {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub amount: u64,
    pub payer: Pubkey,
    /// Slippage tolerance applied to this specific swap, in basis points.
    pub slippage_bps: u32,
}

/// A fully-built, ready-to-sign swap: the route it was derived from, plus
/// the concrete instructions to execute it.
#[derive(Debug, Clone)]
pub struct SwapPlan {
    pub route: Quote,
    pub instructions: Vec<Instruction>,
}

/// Available liquidity for a market.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Liquidity {
    pub base_reserve: u64,
    pub quote_reserve: u64,
    pub timestamp: DateTime<Utc>,
}

/// A price update pushed from a DEX's `subscribe_prices` stream.
#[derive(Debug, Clone)]
pub struct PriceUpdate {
    pub dex: String,
    pub market: Pubkey,
    pub price: f64,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_quote() -> Quote {
        Quote {
            in_amount: 1_000_000,
            out_amount: 2_000_000,
            fee_amount: 2_500,
            fee_bps: 25,
            price_impact: 0.001,
            liquidity: 10_000_000,
            route: vec![],
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_min_out_amount() {
        let quote = sample_quote();
        // 2,000,000 out, 50 bps (0.5%) slippage -> 1,990,000
        assert_eq!(quote.min_out_amount(50), 1_990_000);
    }

    #[test]
    fn test_min_out_amount_zero_slippage() {
        let quote = sample_quote();
        assert_eq!(quote.min_out_amount(0), quote.out_amount);
    }

    #[test]
    fn test_min_out_amount_clamps_over_100_percent() {
        let quote = sample_quote();
        assert_eq!(quote.min_out_amount(20_000), 0);
    }

    #[test]
    fn test_price() {
        let quote = sample_quote();
        assert_eq!(quote.price(), 2.0);
    }

    #[test]
    fn test_price_zero_input() {
        let mut quote = sample_quote();
        quote.in_amount = 0;
        assert_eq!(quote.price(), 0.0);
    }
}
