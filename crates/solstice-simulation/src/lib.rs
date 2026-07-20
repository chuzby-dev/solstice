//! Solstice Simulation
//!
//! Paper trading engine: live on-chain quotes -> strategy evaluation ->
//! sized, risk-checked, simulated fills. No real transactions are ever
//! built or submitted.

pub mod engine;
pub mod error;

pub use engine::{MonitoredPair, PaperTradingConfig, PaperTradingEngine};
pub use error::{SimulationError, SimulationResult};
