//! Phase 5: Jito bundle submission for MEV-protected execution.
//!
//! This module is a **transport layer**, not a swap-building one: it
//! bundles, tips, submits, and confirms already-signed transactions,
//! regardless of what those transactions do. Nothing in this workspace
//! currently constructs a real DEX swap instruction (`solstice-dex`'s
//! `Quote`/`RouteSegment` carry pricing/routing metadata only — no program
//! ID or account list), so end-to-end "buy SOL via a Jito-protected
//! Raydium swap" is not something this module can do by itself yet. See
//! `docs/CHANGELOG.md`'s Phase 5 entry for the full reasoning.

pub mod bundle;
pub mod client;
pub mod error;
pub mod fallback;
pub mod tip;

pub use bundle::{Bundle, BundleStatus, MAX_BUNDLE_TRANSACTIONS};
pub use client::{JitoClient, JitoConfig};
pub use error::{JitoError, JitoResult};
pub use fallback::{submit_with_fallback, SubmissionMethod, SubmissionOutcome};
pub use tip::{build_tip_instruction, TipStrategy};
