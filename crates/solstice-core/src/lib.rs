//! Solstice Core Types and Abstractions
//!
//! This crate provides the foundational types, traits, and abstractions used throughout
//! the Solstice trading platform. All crates depend on solstice-core for type safety and
//! consistent interfaces.

pub mod error;
pub mod types;
pub mod logging;

pub use error::{Result, SolsticeError};
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crate_imports() {
        // Verify core types are accessible
        let _ = std::marker::PhantomData::<Price>;
        let _ = std::marker::PhantomData::<Position>;
    }
}
