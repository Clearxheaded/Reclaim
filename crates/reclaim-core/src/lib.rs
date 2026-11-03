//! # Reclaim Core
//!
//! `reclaim-core` is the foundational library for Reclaim.
//!
//! In Milestone 1, it provides a safe, read-only, non-following filesystem scanner.
//!
//! ## Safety Guarantees
//! - Purely read-only: does not modify, move, or delete files.
//! - Non-following: never traverses symbolic links or Windows junctions/reparse points.
//! - Stack-safe: uses explicit heap-allocated traversal to prevent call-stack overflows.
//! - Resilient: collects permissions and I/O errors without aborting the scan.
//! - Responsive: supports thread-safe cancellation and progress reporting.

pub mod duplicates;
pub mod entry;
pub mod error;
pub mod file_id;
pub mod planner;
pub mod platform;
pub mod scanner;

// Convenient re-exports for library consumers
pub use duplicates::{
    find_duplicates, DuplicateFileError, DuplicateGroup, DuplicateOptions, DuplicateOutput,
    DuplicateProgress, DuplicateProgressCallback, DuplicateStage, DuplicateSummary, PhysicalCopy,
    DEFAULT_PREFIX_SIZE, HASH_BUFFER_SIZE,
};
pub use entry::{FileEntry, SkipReason, SkippedEntry};
pub use error::{RootScanError, ScanError, ScanErrorKind};
pub use file_id::FileId;
pub use planner::{
    generate_plan, ActionPreconditions, CleanupPlan, KeeperPolicy, PlanError, PlanOptions,
    PlanSummary, ProtectedKeeper, QuarantineAction,
};
pub use scanner::{
    scan_directory, CancellationToken, ProgressCallback, ScanOptions, ScanOutput, ScanProgress,
    ScanSummary,
};
