//! Durable recovery and finalization orchestration.
//!
//! Recovery deliberately has no source URL or browser state of its own.  It
//! records what must be reconciled and lets the feed/detail workers reacquire
//! current data after a restart.

pub mod finalizer;
pub mod manager;

pub use finalizer::{FinalizationError, FinalizationPhase, Finalizer};
pub use manager::{RecoveryConfig, RecoveryJob, RecoveryManager, RecoveryOutcome};
