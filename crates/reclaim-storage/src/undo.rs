//! # Safe Conditional Undo Engine
//!
//! Implements Milestone 6: conditional reversal of completed quarantine actions.
//! Safely restores staged duplicates back to their original locations while rigorously
//! enforcing non-negotiable safety invariants:
//!
//! ## Non-Negotiable Safety Invariants
//! 1. **Honest Undo Guarantees (Conditional Reversal)**:
//!    - A file is only restored if:
//!      - The original destination path is strictly vacant (no TOCTOU replacement!).
//!      - The preserved keeper file still exists and its content/identity matches.
//!      - The quarantined file still exists and its cryptographic BLAKE3 content hash
//!        matches the recorded preconditions.
//!    - **Never overwrite an existing file merely to complete an undo.** If the original path
//!      is occupied, an explicit conflict is logged and the quarantined file remains safe.
//! 2. **Durable State Transition to `Reverted`**:
//!    - Successfully undone actions transition `Completed` -> `Reverted` in SQLite with
//!      timestamp and audit trail event.
//!    - **Prevent Double-Undo**: Actions already in `Reverted` state cannot be undone again.
//! 3. **Fail-Closed on Collision**:
//!    - Kernel-level atomic non-replacing move (`atomic_move_no_replace`) guarantees zero
//!      race windows between vacancy validation and file restoration.

use crate::error::StorageError;
use crate::journal::ActionJournal;
use crate::types::{ActionId, ActionState, JournaledAction, PlanId, SessionId};
use reclaim_core::mutation::{
    atomic_move_no_replace, compute_file_blake3_hash, verify_same_filesystem, MutationError,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Configuration options controlling undo execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UndoOptions {
    /// When true, performs complete pre-reversion validation without moving files or updating journal state.
    pub dry_run: bool,
    /// When true, halts execution on the first conflict or error rather than continuing with remaining actions.
    pub halt_on_error: bool,
}

/// The specific reason an action could not be undone or restored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoConflictReason {
    /// The original destination path is already occupied by an existing file, directory, or symlink.
    DestinationOccupied { path: PathBuf },
    /// The quarantined file is missing from the staged quarantine directory.
    QuarantinedFileMissing { path: PathBuf },
    /// The quarantined file is a symbolic link or redirect; refusing to accept.
    QuarantinedFileIsSymlink { path: PathBuf },
    /// The quarantined file is not a regular file.
    QuarantinedFileNotRegular { path: PathBuf },
    /// Quarantined file size does not match recorded precondition size.
    QuarantinedFileSizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    /// Quarantined file cryptographic BLAKE3 content hash does not match recorded precondition hash.
    QuarantinedFileHashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    /// The preserved keeper file was deleted or cannot be found.
    KeeperMissing { path: PathBuf },
    /// The preserved keeper file is a symbolic link.
    KeeperIsSymlink { path: PathBuf },
    /// The preserved keeper file is not a regular file.
    KeeperNotRegular { path: PathBuf },
    /// Preserved keeper file size changed.
    KeeperSizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    /// Preserved keeper file BLAKE3 content hash changed.
    KeeperHashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    /// Quarantine staging directory and destination reside on different filesystem devices.
    CrossDeviceMoveUnsupported {
        quarantine: PathBuf,
        destination: PathBuf,
    },
    /// Action is not in Completed state (e.g. already Reverted, Failed, or Planned).
    InvalidActionState {
        action_id: ActionId,
        state: ActionState,
    },
    /// Low-level filesystem mutation or I/O error.
    Io { path: PathBuf, message: String },
}

impl fmt::Display for UndoConflictReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UndoConflictReason::DestinationOccupied { path } => {
                write!(
                    f,
                    "Destination path already exists; refusing to overwrite: '{}'",
                    path.display()
                )
            }
            UndoConflictReason::QuarantinedFileMissing { path } => {
                write!(
                    f,
                    "Quarantined file missing from staging folder: '{}'",
                    path.display()
                )
            }
            UndoConflictReason::QuarantinedFileIsSymlink { path } => {
                write!(
                    f,
                    "Quarantined file is a symbolic link; refusing to accept: '{}'",
                    path.display()
                )
            }
            UndoConflictReason::QuarantinedFileNotRegular { path } => {
                write!(
                    f,
                    "Quarantined file is not a regular file: '{}'",
                    path.display()
                )
            }
            UndoConflictReason::QuarantinedFileSizeMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Quarantined file size mismatch for '{}': expected {} bytes, found {} bytes",
                    path.display(),
                    expected,
                    actual
                )
            }
            UndoConflictReason::QuarantinedFileHashMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Quarantined file hash mismatch for '{}': expected {}, found {}",
                    path.display(),
                    expected,
                    actual
                )
            }
            UndoConflictReason::KeeperMissing { path } => {
                write!(
                    f,
                    "Preserved keeper file missing: '{}'; cannot verify duplicate safety",
                    path.display()
                )
            }
            UndoConflictReason::KeeperIsSymlink { path } => {
                write!(
                    f,
                    "Preserved keeper file is a symbolic link: '{}'",
                    path.display()
                )
            }
            UndoConflictReason::KeeperNotRegular { path } => {
                write!(
                    f,
                    "Preserved keeper file is not a regular file: '{}'",
                    path.display()
                )
            }
            UndoConflictReason::KeeperSizeMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Preserved keeper file size mismatch for '{}': expected {} bytes, found {} bytes",
                    path.display(),
                    expected,
                    actual
                )
            }
            UndoConflictReason::KeeperHashMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Preserved keeper file hash mismatch for '{}': expected {}, found {}",
                    path.display(),
                    expected,
                    actual
                )
            }
            UndoConflictReason::CrossDeviceMoveUnsupported {
                quarantine,
                destination,
            } => {
                write!(
                    f,
                    "Cross-device move unsupported between '{}' and '{}'",
                    quarantine.display(),
                    destination.display()
                )
            }
            UndoConflictReason::InvalidActionState { action_id, state } => {
                write!(
                    f,
                    "Action '{}' is in '{}' state; only 'Completed' actions can be reverted",
                    action_id, state
                )
            }
            UndoConflictReason::Io { path, message } => {
                write!(
                    f,
                    "Filesystem I/O error at '{}': {}",
                    path.display(),
                    message
                )
            }
        }
    }
}

/// Detailed conflict record for an action that could not be undone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoConflictDetail {
    /// Unique action identifier in the durable journal.
    pub action_id: ActionId,
    /// Destination path where the file would have been restored.
    pub source_path: PathBuf,
    /// Current quarantine path holding the staged duplicate.
    pub quarantine_path: PathBuf,
    /// Preserved keeper path preserving this content.
    pub keeper_path: PathBuf,
    /// Detailed conflict explanation.
    pub reason: UndoConflictReason,
}

/// Outcome status of an individual action during an undo run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoStatus {
    /// File was successfully restored to original path and action transitioned to Reverted.
    Restored,
    /// Pre-reversion validation succeeded in dry-run mode (zero files modified).
    DryRunValidated,
    /// Restoration could not proceed due to a safety conflict.
    Conflict { reason: UndoConflictReason },
    /// Action was skipped (e.g. already Reverted or not in Completed state).
    Skipped { reason: String },
}

/// Detailed result of evaluating or executing an individual undo action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoActionResult {
    /// Unique action identifier.
    pub action_id: ActionId,
    /// Destination path where the file is restored.
    pub source_path: PathBuf,
    /// Quarantined staging path.
    pub quarantine_path: PathBuf,
    /// Preserved keeper path.
    pub keeper_path: PathBuf,
    /// Outcome status.
    pub status: UndoStatus,
    /// Bytes restored (0 if conflicted or skipped).
    pub bytes_restored: u64,
}

/// Comprehensive report summarizing an undo execution run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoReport {
    /// The session identifier targeted, if applicable.
    pub session_id: Option<SessionId>,
    /// The plan identifier targeted, if applicable.
    pub plan_id: Option<PlanId>,
    /// Whether this run was a dry run.
    pub dry_run: bool,
    /// Total actions evaluated during this undo operation.
    pub total_actions_considered: usize,
    /// Actions successfully restored (or dry-run validated).
    pub actions_restored: usize,
    /// Actions that failed pre-reversion validation due to safety conflicts.
    pub actions_conflicted: usize,
    /// Actions that were skipped (e.g. already Reverted).
    pub actions_skipped: usize,
    /// Total bytes restored to original locations.
    pub bytes_restored: u64,
    /// Conflicts encountered during the run.
    pub conflicts: Vec<UndoConflictDetail>,
    /// Individual action results.
    pub results: Vec<UndoActionResult>,
    /// Wall-clock duration elapsed.
    pub elapsed: Duration,
}

/// Event emitted during undo execution for real-time terminal progress monitoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoProgressEvent<'a> {
    /// Pre-reversion validation started.
    Validating {
        action_id: ActionId,
        source: &'a PathBuf,
        quarantine: &'a PathBuf,
        index: usize,
        total: usize,
    },
    /// File restoration actively in-flight.
    Restoring {
        action_id: ActionId,
        source: &'a PathBuf,
        quarantine: &'a PathBuf,
        dry_run: bool,
    },
    /// File restored successfully.
    Restored {
        action_id: ActionId,
        source: &'a PathBuf,
        bytes_restored: u64,
        dry_run: bool,
    },
    /// Safety conflict detected; restoration refused.
    Conflict {
        action_id: ActionId,
        source: &'a PathBuf,
        reason: &'a UndoConflictReason,
    },
    /// Action skipped.
    Skipped {
        action_id: ActionId,
        source: &'a PathBuf,
        reason: &'a str,
    },
}

/// Safe undo execution runner enforcing conditional reversal and fail-closed safety.
#[derive(Debug, Clone, Default)]
pub struct SafeReverter {
    options: UndoOptions,
}

impl SafeReverter {
    /// Creates a new `SafeReverter` with the specified options.
    pub fn new(options: UndoOptions) -> Self {
        Self { options }
    }

    /// Returns the active undo options.
    pub fn options(&self) -> &UndoOptions {
        &self.options
    }

    /// Reverts all Completed actions in a specific session.
    pub fn revert_session(
        &self,
        journal: &mut ActionJournal,
        session_id: SessionId,
    ) -> Result<UndoReport, StorageError> {
        self.revert_session_with_callback(journal, session_id, |_| {})
    }

    /// Reverts all Completed actions in a specific session with a progress callback.
    pub fn revert_session_with_callback<F>(
        &self,
        journal: &mut ActionJournal,
        session_id: SessionId,
        mut callback: F,
    ) -> Result<UndoReport, StorageError>
    where
        F: FnMut(UndoProgressEvent),
    {
        let actions = journal.list_actions_for_session(session_id)?;
        self.revert_actions_internal(journal, Some(session_id), None, actions, &mut callback)
    }

    /// Reverts all Completed actions in a specific plan.
    pub fn revert_plan(
        &self,
        journal: &mut ActionJournal,
        plan_id: PlanId,
    ) -> Result<UndoReport, StorageError> {
        self.revert_plan_with_callback(journal, plan_id, |_| {})
    }

    /// Reverts all Completed actions in a specific plan with a progress callback.
    pub fn revert_plan_with_callback<F>(
        &self,
        journal: &mut ActionJournal,
        plan_id: PlanId,
        mut callback: F,
    ) -> Result<UndoReport, StorageError>
    where
        F: FnMut(UndoProgressEvent),
    {
        let actions = journal.list_actions_for_plan(plan_id)?;
        let plan_details = journal.fetch_plan(plan_id)?;
        self.revert_actions_internal(
            journal,
            Some(plan_details.session_id),
            Some(plan_id),
            actions,
            &mut callback,
        )
    }

    /// Reverts a specific list of actions.
    pub fn revert_actions(
        &self,
        journal: &mut ActionJournal,
        actions: Vec<JournaledAction>,
    ) -> Result<UndoReport, StorageError> {
        self.revert_actions_with_callback(journal, actions, |_| {})
    }

    /// Reverts a specific list of actions with a progress callback.
    pub fn revert_actions_with_callback<F>(
        &self,
        journal: &mut ActionJournal,
        actions: Vec<JournaledAction>,
        mut callback: F,
    ) -> Result<UndoReport, StorageError>
    where
        F: FnMut(UndoProgressEvent),
    {
        self.revert_actions_internal(journal, None, None, actions, &mut callback)
    }

    /// Core engine implementing pre-reversion validation and safe atomic restoration.
    fn revert_actions_internal<F>(
        &self,
        journal: &mut ActionJournal,
        session_id: Option<SessionId>,
        plan_id: Option<PlanId>,
        actions: Vec<JournaledAction>,
        callback: &mut F,
    ) -> Result<UndoReport, StorageError>
    where
        F: FnMut(UndoProgressEvent),
    {
        let start_time = Instant::now();
        let total_considered = actions.len();
        let mut actions_restored = 0;
        let mut actions_conflicted = 0;
        let mut actions_skipped = 0;
        let mut bytes_restored = 0u64;
        let mut conflicts = Vec::new();
        let mut results = Vec::new();

        for (idx, action) in actions.into_iter().enumerate() {
            // 1. Check current action state
            if action.state == ActionState::Reverted {
                // Prevent double-undo: an action already in Reverted cannot be undone again
                let reason = "Action already reverted; skipping double-undo".to_string();
                callback(UndoProgressEvent::Skipped {
                    action_id: action.action_id,
                    source: &action.source_path,
                    reason: &reason,
                });
                actions_skipped += 1;
                results.push(UndoActionResult {
                    action_id: action.action_id,
                    source_path: action.source_path,
                    quarantine_path: action.quarantine_path,
                    keeper_path: action.keeper_path,
                    status: UndoStatus::Skipped { reason },
                    bytes_restored: 0,
                });
                continue;
            }

            if action.state != ActionState::Completed {
                let reason = format!(
                    "Action is in '{}' state; only 'Completed' actions can be reverted",
                    action.state
                );
                callback(UndoProgressEvent::Skipped {
                    action_id: action.action_id,
                    source: &action.source_path,
                    reason: &reason,
                });
                actions_skipped += 1;
                results.push(UndoActionResult {
                    action_id: action.action_id,
                    source_path: action.source_path,
                    quarantine_path: action.quarantine_path,
                    keeper_path: action.keeper_path,
                    status: UndoStatus::Skipped { reason },
                    bytes_restored: 0,
                });
                continue;
            }

            // 2. Pre-reversion validation
            callback(UndoProgressEvent::Validating {
                action_id: action.action_id,
                source: &action.source_path,
                quarantine: &action.quarantine_path,
                index: idx + 1,
                total: total_considered,
            });

            match validate_reversion_preconditions(&action) {
                Err(conflict_reason) => {
                    // Precondition failed: log conflict, refuse move, preserve quarantine file
                    callback(UndoProgressEvent::Conflict {
                        action_id: action.action_id,
                        source: &action.source_path,
                        reason: &conflict_reason,
                    });

                    // Log audit entry in SQLite journal
                    let audit_note = format!("Undo conflict: {}", conflict_reason);
                    let _ = journal.log_action_event(action.action_id, action.state, &audit_note);

                    conflicts.push(UndoConflictDetail {
                        action_id: action.action_id,
                        source_path: action.source_path.clone(),
                        quarantine_path: action.quarantine_path.clone(),
                        keeper_path: action.keeper_path.clone(),
                        reason: conflict_reason.clone(),
                    });

                    actions_conflicted += 1;
                    results.push(UndoActionResult {
                        action_id: action.action_id,
                        source_path: action.source_path,
                        quarantine_path: action.quarantine_path,
                        keeper_path: action.keeper_path,
                        status: UndoStatus::Conflict {
                            reason: conflict_reason,
                        },
                        bytes_restored: 0,
                    });

                    if self.options.halt_on_error {
                        break;
                    }
                }
                Ok(()) => {
                    // Preconditions passed!
                    if self.options.dry_run {
                        // Dry-run simulation: zero file mutations, zero journal mutations
                        callback(UndoProgressEvent::Restored {
                            action_id: action.action_id,
                            source: &action.source_path,
                            bytes_restored: action.preconditions.expected_size,
                            dry_run: true,
                        });

                        actions_restored += 1;
                        bytes_restored =
                            bytes_restored.saturating_add(action.preconditions.expected_size);
                        results.push(UndoActionResult {
                            action_id: action.action_id,
                            source_path: action.source_path,
                            quarantine_path: action.quarantine_path,
                            keeper_path: action.keeper_path,
                            status: UndoStatus::DryRunValidated,
                            bytes_restored: action.preconditions.expected_size,
                        });
                    } else {
                        // Live restoration: atomically move file back to original location
                        callback(UndoProgressEvent::Restoring {
                            action_id: action.action_id,
                            source: &action.source_path,
                            quarantine: &action.quarantine_path,
                            dry_run: false,
                        });

                        // Create destination parent directory if missing
                        if let Some(parent) = action.source_path.parent() {
                            if let Err(e) = fs::create_dir_all(parent) {
                                let conflict_reason = UndoConflictReason::Io {
                                    path: parent.to_path_buf(),
                                    message: format!(
                                        "Failed to create restoration directory: {}",
                                        e
                                    ),
                                };
                                callback(UndoProgressEvent::Conflict {
                                    action_id: action.action_id,
                                    source: &action.source_path,
                                    reason: &conflict_reason,
                                });
                                conflicts.push(UndoConflictDetail {
                                    action_id: action.action_id,
                                    source_path: action.source_path.clone(),
                                    quarantine_path: action.quarantine_path.clone(),
                                    keeper_path: action.keeper_path.clone(),
                                    reason: conflict_reason.clone(),
                                });
                                actions_conflicted += 1;
                                results.push(UndoActionResult {
                                    action_id: action.action_id,
                                    source_path: action.source_path,
                                    quarantine_path: action.quarantine_path,
                                    keeper_path: action.keeper_path,
                                    status: UndoStatus::Conflict {
                                        reason: conflict_reason,
                                    },
                                    bytes_restored: 0,
                                });
                                if self.options.halt_on_error {
                                    break;
                                }
                                continue;
                            }
                        }

                        // Atomic non-replacing move back to original path
                        match atomic_move_no_replace(&action.quarantine_path, &action.source_path) {
                            Ok(()) => {
                                // Transition action state to Reverted in the durable journal
                                journal.record_action_transition(
                                    action.action_id,
                                    ActionState::Reverted,
                                    Some("Restored from quarantine during undo"),
                                )?;

                                // Try cleaning up empty quarantine directory if left empty
                                if let Some(q_parent) = action.quarantine_path.parent() {
                                    let _ = fs::remove_dir(q_parent);
                                }

                                callback(UndoProgressEvent::Restored {
                                    action_id: action.action_id,
                                    source: &action.source_path,
                                    bytes_restored: action.preconditions.expected_size,
                                    dry_run: false,
                                });

                                actions_restored += 1;
                                bytes_restored = bytes_restored
                                    .saturating_add(action.preconditions.expected_size);
                                results.push(UndoActionResult {
                                    action_id: action.action_id,
                                    source_path: action.source_path,
                                    quarantine_path: action.quarantine_path,
                                    keeper_path: action.keeper_path,
                                    status: UndoStatus::Restored,
                                    bytes_restored: action.preconditions.expected_size,
                                });
                            }
                            Err(MutationError::DestinationAlreadyExists(dest)) => {
                                // Kernel detected destination collision (TOCTOU race defense)
                                let conflict_reason =
                                    UndoConflictReason::DestinationOccupied { path: dest };
                                callback(UndoProgressEvent::Conflict {
                                    action_id: action.action_id,
                                    source: &action.source_path,
                                    reason: &conflict_reason,
                                });

                                let audit_note = format!(
                                    "Undo conflict during atomic move: {}",
                                    conflict_reason
                                );
                                let _ = journal.log_action_event(
                                    action.action_id,
                                    action.state,
                                    &audit_note,
                                );

                                conflicts.push(UndoConflictDetail {
                                    action_id: action.action_id,
                                    source_path: action.source_path.clone(),
                                    quarantine_path: action.quarantine_path.clone(),
                                    keeper_path: action.keeper_path.clone(),
                                    reason: conflict_reason.clone(),
                                });
                                actions_conflicted += 1;
                                results.push(UndoActionResult {
                                    action_id: action.action_id,
                                    source_path: action.source_path,
                                    quarantine_path: action.quarantine_path,
                                    keeper_path: action.keeper_path,
                                    status: UndoStatus::Conflict {
                                        reason: conflict_reason,
                                    },
                                    bytes_restored: 0,
                                });

                                if self.options.halt_on_error {
                                    break;
                                }
                            }
                            Err(e) => {
                                let conflict_reason = UndoConflictReason::Io {
                                    path: action.quarantine_path.clone(),
                                    message: format!("Failed atomic move during undo: {}", e),
                                };
                                callback(UndoProgressEvent::Conflict {
                                    action_id: action.action_id,
                                    source: &action.source_path,
                                    reason: &conflict_reason,
                                });

                                conflicts.push(UndoConflictDetail {
                                    action_id: action.action_id,
                                    source_path: action.source_path.clone(),
                                    quarantine_path: action.quarantine_path.clone(),
                                    keeper_path: action.keeper_path.clone(),
                                    reason: conflict_reason.clone(),
                                });
                                actions_conflicted += 1;
                                results.push(UndoActionResult {
                                    action_id: action.action_id,
                                    source_path: action.source_path,
                                    quarantine_path: action.quarantine_path,
                                    keeper_path: action.keeper_path,
                                    status: UndoStatus::Conflict {
                                        reason: conflict_reason,
                                    },
                                    bytes_restored: 0,
                                });

                                if self.options.halt_on_error {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(UndoReport {
            session_id,
            plan_id,
            dry_run: self.options.dry_run,
            total_actions_considered: total_considered,
            actions_restored,
            actions_conflicted,
            actions_skipped,
            bytes_restored,
            conflicts,
            results,
            elapsed: start_time.elapsed(),
        })
    }
}

/// Rigorously validates all preconditions required for safe conditional undo.
///
/// Precondition Checks:
/// 1. Preserved Keeper: must exist, must not be symlink, must be regular file, size and BLAKE3 hash must match.
/// 2. Quarantined Duplicate: must exist, must not be symlink, must be regular file, size and BLAKE3 hash must match.
/// 3. Destination Vacancy: original source path must NOT exist (fails closed if path occupied).
/// 4. Filesystem Boundaries: quarantine staging and restoration destination must reside on the same filesystem.
fn validate_reversion_preconditions(action: &JournaledAction) -> Result<(), UndoConflictReason> {
    // 1. Quarantined file verification
    let q_meta = match fs::symlink_metadata(&action.quarantine_path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(UndoConflictReason::QuarantinedFileMissing {
                path: action.quarantine_path.clone(),
            });
        }
        Err(e) => {
            return Err(UndoConflictReason::Io {
                path: action.quarantine_path.clone(),
                message: e.to_string(),
            });
        }
    };

    if q_meta.file_type().is_symlink() {
        return Err(UndoConflictReason::QuarantinedFileIsSymlink {
            path: action.quarantine_path.clone(),
        });
    }
    if !q_meta.file_type().is_file() {
        return Err(UndoConflictReason::QuarantinedFileNotRegular {
            path: action.quarantine_path.clone(),
        });
    }
    if q_meta.len() != action.preconditions.expected_size {
        return Err(UndoConflictReason::QuarantinedFileSizeMismatch {
            path: action.quarantine_path.clone(),
            expected: action.preconditions.expected_size,
            actual: q_meta.len(),
        });
    }
    match compute_file_blake3_hash(&action.quarantine_path) {
        Ok(hash) => {
            if hash != action.preconditions.expected_hash {
                return Err(UndoConflictReason::QuarantinedFileHashMismatch {
                    path: action.quarantine_path.clone(),
                    expected: action.preconditions.expected_hash.clone(),
                    actual: hash,
                });
            }
        }
        Err(e) => {
            return Err(UndoConflictReason::Io {
                path: action.quarantine_path.clone(),
                message: format!("Failed to compute BLAKE3 hash: {}", e),
            });
        }
    }

    // 2. Preserved keeper file verification
    let k_meta = match fs::symlink_metadata(&action.keeper_path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(UndoConflictReason::KeeperMissing {
                path: action.keeper_path.clone(),
            });
        }
        Err(e) => {
            return Err(UndoConflictReason::Io {
                path: action.keeper_path.clone(),
                message: e.to_string(),
            });
        }
    };

    if k_meta.file_type().is_symlink() {
        return Err(UndoConflictReason::KeeperIsSymlink {
            path: action.keeper_path.clone(),
        });
    }
    if !k_meta.file_type().is_file() {
        return Err(UndoConflictReason::KeeperNotRegular {
            path: action.keeper_path.clone(),
        });
    }
    if k_meta.len() != action.preconditions.expected_size {
        return Err(UndoConflictReason::KeeperSizeMismatch {
            path: action.keeper_path.clone(),
            expected: action.preconditions.expected_size,
            actual: k_meta.len(),
        });
    }
    match compute_file_blake3_hash(&action.keeper_path) {
        Ok(hash) => {
            if hash != action.preconditions.expected_hash {
                return Err(UndoConflictReason::KeeperHashMismatch {
                    path: action.keeper_path.clone(),
                    expected: action.preconditions.expected_hash.clone(),
                    actual: hash,
                });
            }
        }
        Err(e) => {
            return Err(UndoConflictReason::Io {
                path: action.keeper_path.clone(),
                message: format!("Failed to compute keeper BLAKE3 hash: {}", e),
            });
        }
    }

    // 3. Destination vacancy check: source path must NOT exist
    match fs::symlink_metadata(&action.source_path) {
        Ok(_) => {
            // Path is occupied! Never overwrite an existing destination during undo
            return Err(UndoConflictReason::DestinationOccupied {
                path: action.source_path.clone(),
            });
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Path is vacant; safe to restore
        }
        Err(e) => {
            return Err(UndoConflictReason::Io {
                path: action.source_path.clone(),
                message: e.to_string(),
            });
        }
    }

    // 4. Same filesystem boundary check
    if let Err(e) = verify_same_filesystem(&action.quarantine_path, &action.source_path) {
        match e {
            MutationError::CrossDeviceMoveUnsupported {
                source,
                destination,
            } => {
                return Err(UndoConflictReason::CrossDeviceMoveUnsupported {
                    quarantine: source,
                    destination,
                });
            }
            other => {
                return Err(UndoConflictReason::Io {
                    path: action.source_path.clone(),
                    message: other.to_string(),
                });
            }
        }
    }

    Ok(())
}
