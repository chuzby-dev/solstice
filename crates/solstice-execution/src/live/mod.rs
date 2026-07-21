//! Automated live trading (Phase: "wire it up"): the same signal → size →
//! risk-check pipeline as paper trading, gated behind an explicit,
//! runtime-toggleable kill switch that defaults to off. See
//! [`engine::LiveTradingEngine`]'s doc comment for the full design.

pub mod config;
pub mod engine;

pub use config::{LiveTradedPair, LiveTradingConfig};
pub use engine::{LiveEvent, LivePositionSnapshot, LiveStatusSnapshot, LiveTradingEngine};
