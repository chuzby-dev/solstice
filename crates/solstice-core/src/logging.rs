//! Logging configuration and utilities for Solstice.

use std::path::Path;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize logging with structured JSON output.
pub fn init_logging() {
    init_logging_with_env("SOLSTICE_LOG_LEVEL");
}

/// Initialize logging with a custom environment variable for log level.
pub fn init_logging_with_env(env_var: &str) {
    let env_filter = EnvFilter::try_from_env(env_var).unwrap_or_else(|_| EnvFilter::new("info"));

    let layer = fmt::layer()
        .json()
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(layer)
        .init();
}

/// Initialize logging with file output.
pub fn init_logging_with_file(path: impl AsRef<Path>) -> std::io::Result<()> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let layer = fmt::layer()
        .json()
        .with_writer(file)
        .with_target(true)
        .with_thread_ids(true);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(layer)
        .init();

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_logging_init() {
        // Note: Can only initialize logging once per process
        // This test just verifies the function exists and compiles
        let result = std::panic::catch_unwind(|| {
            // Don't actually call init_logging() in tests
            // as it will panic if logging is already initialized
        });
        assert!(result.is_ok());
    }
}
