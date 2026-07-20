//! Yellowstone gRPC adapter: real-time Solana account state streaming.
//!
//! See `docs/YELLOWSTONE.md` for the architecture this implements.

pub mod client;
pub mod config;
pub mod filter;
pub mod parser;

pub use client::YellowstoneClient;
pub use config::YellowstoneConfig;
pub use filter::AccountFilter;
pub use parser::YellowstoneParser;
