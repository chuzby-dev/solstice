//! Solstice Simulation
//!
//! Paper trading engine: live on-chain quotes -> strategy evaluation ->
//! sized, risk-checked, simulated fills. No real transactions are ever
//! built or submitted.

pub mod backtest;
pub mod demo;
pub mod engine;
pub mod error;

pub use demo::build_sol_usdc_demo_engine;
pub use engine::{
    EngineEvent, MonitoredPair, PaperTradingConfig, PaperTradingEngine, PortfolioSnapshot,
    PositionSnapshot,
};
pub use error::{SimulationError, SimulationResult};
