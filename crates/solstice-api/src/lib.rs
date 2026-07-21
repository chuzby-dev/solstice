//! Solstice API
//!
//! REST + WebSocket server exposing a running [`solstice_simulation::PaperTradingEngine`]'s
//! state: status, positions, trades, performance, and a real-time event
//! stream. No authentication yet — intended for local/trusted-network use
//! only (see `docs/CHANGELOG.md` for why).

pub mod dto;
pub mod error;
pub mod handlers;
pub mod server;
pub mod state;
pub mod websocket;

pub use error::{ApiError, ApiResult};
pub use server::ApiServer;
pub use state::AppState;

#[cfg(test)]
mod tests {
    #[test]
    fn test_crate_imports() {
        let _ = std::marker::PhantomData::<super::ApiServer>;
    }
}
