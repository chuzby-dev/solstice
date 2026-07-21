//! Shared application state, threaded through every Axum handler.

use solstice_simulation::PaperTradingEngine;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<PaperTradingEngine>,
}

impl AppState {
    pub fn new(engine: Arc<PaperTradingEngine>) -> Self {
        AppState { engine }
    }
}
