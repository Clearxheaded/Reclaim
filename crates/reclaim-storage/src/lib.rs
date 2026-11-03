//! # Reclaim Storage & Durable Journal Crate
//!
//! Provides a crash-resilient, SQLite-backed relational journal and audit store
//! for tracking Reclaim sessions, cleanup plans, action state machines, and preconditions.
//!
//! ## Core Architecture
//! - **Relational Storage**: Backed by `rusqlite` with WAL (`Write-Ahead Logging`) and foreign key cascades.
//! - **State Machine Tracking**: Actions transition strictly across `Planned` -> `Validating` -> `Executing` -> `Completed` | `Failed` | `Reverted`.
//! - **Audit Logging**: Every state transition is recorded chronologically in an immutable event trail.
//! - **Crash Recovery**: Detects interrupted sessions and dangling `Validating`/`Executing` actions on startup.

pub mod datetime;
pub mod error;
pub mod executor;
pub mod journal;
pub mod recovery;
pub mod schema;
pub mod types;
pub mod undo;

// Re-export primary types for ergonomic usage
pub use error::StorageError;
pub use executor::{
    ActionExecutionStatus, ActionResult, ExecutionOptions, ExecutionProgressEvent, ExecutionReport,
    SafeExecutor,
};
pub use journal::ActionJournal;
pub use recovery::{
    count_dangling_sessions, inspect_startup_state, reconcile_interrupted_sessions,
    reconcile_startup_state, ActionRecoveryDetail, ActionRecoveryResolution, RecoveryReport,
    StartupRecoveryReport,
};
pub use schema::CURRENT_SCHEMA_VERSION;
pub use types::{
    ActionId, ActionState, JournalEvent, JournaledAction, PlanDetails, PlanId, SessionDetails,
    SessionHistorySummary, SessionId, SessionStatus,
};
pub use undo::{
    SafeReverter, UndoActionResult, UndoConflictDetail, UndoConflictReason, UndoOptions,
    UndoProgressEvent, UndoReport, UndoStatus,
};
