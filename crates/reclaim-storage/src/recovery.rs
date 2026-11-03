//! Crash inspection, recovery assessment, and active filesystem reconciliation.
//!
//! Evaluates journal and physical filesystem state on application startup or recovery
//! to detect unclean terminations, reconcile dangling in-flight mutations, and safely
//! resolve interrupted sessions.

use crate::error::StorageError;
use crate::journal::ActionJournal;
use crate::types::{ActionId, ActionState, JournaledAction, PlanId, SessionDetails, SessionStatus};
use reclaim_core::mutation::compute_file_blake3_hash;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::PathBuf;

/// Physical filesystem outcome determined during crash reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionRecoveryResolution {
    /// File was verified at the quarantine destination with matching size and hash.
    /// The atomic move succeeded before the crash; transitioned to Completed.
    CompletedAtQuarantine,
    /// File remains intact at source path with matching size and hash.
    /// The move never executed before the crash; transitioned to Failed.
    UnmovedAtSource,
    /// Content mismatch, corruption, or missing at both paths; transitioned to Failed.
    CorruptOrMissing { reason: String },
}

impl fmt::Display for ActionRecoveryResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionRecoveryResolution::CompletedAtQuarantine => {
                write!(
                    f,
                    "Verified in quarantine with matching content; marked Completed"
                )
            }
            ActionRecoveryResolution::UnmovedAtSource => {
                write!(
                    f,
                    "Remains untouched at source with matching content; marked Failed"
                )
            }
            ActionRecoveryResolution::CorruptOrMissing { reason } => {
                write!(f, "Corrupt or missing: {}", reason)
            }
        }
    }
}

/// Detailed record of an individual action reconciled during startup recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRecoveryDetail {
    /// Unique action identifier in the durable journal.
    pub action_id: ActionId,
    /// Parent cleanup plan identifier.
    pub plan_id: PlanId,
    /// Original duplicate source path.
    pub source_path: PathBuf,
    /// Staged quarantine destination path.
    pub quarantine_path: PathBuf,
    /// The state the action was left in before recovery (`Validating` or `Executing`).
    pub initial_state: ActionState,
    /// The reconciled state transitioned into (`Completed` or `Failed`).
    pub final_state: ActionState,
    /// Physical resolution category.
    pub resolution: ActionRecoveryResolution,
    /// Explanatory diagnostic audit record logged in the journal.
    pub note: String,
}

/// Comprehensive report produced by active filesystem recovery reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    /// True if the journal was already clean (no dangling active sessions or incomplete actions).
    pub is_clean: bool,
    /// Number of dangling in-flight actions inspected against disk.
    pub actions_checked: usize,
    /// Actions reconciled as Completed (move verified on disk before interruption).
    pub actions_resolved_completed: usize,
    /// Actions reconciled as Failed (unmoved, corrupt, or missing).
    pub actions_resolved_failed: usize,
    /// Active sessions that were ungracefully interrupted and marked Interrupted.
    pub sessions_interrupted: usize,
    /// Detailed diagnostic record for each reconciled action.
    pub details: Vec<ActionRecoveryDetail>,
}

/// Summary of startup journal inspection status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRecoveryReport {
    /// True if all previous sessions completed cleanly with zero in-flight actions.
    pub is_clean: bool,
    /// Sessions that were left in `Active` status without a clean shutdown.
    pub interrupted_sessions: Vec<SessionDetails>,
    /// In-flight actions that were interrupted mid-operation (`Validating` or `Executing`).
    pub incomplete_actions: Vec<JournaledAction>,
}

/// Inspects the durable journal on startup to assess whether a previous run crashed.
pub fn inspect_startup_state(
    journal: &ActionJournal,
) -> Result<StartupRecoveryReport, StorageError> {
    let incomplete_actions = journal.get_incomplete_actions()?;
    let active_or_interrupted = journal.find_active_or_interrupted_sessions()?;

    let dangling_active: Vec<SessionDetails> = active_or_interrupted
        .into_iter()
        .filter(|s| s.status == SessionStatus::Active)
        .collect();

    let is_clean = incomplete_actions.is_empty() && dangling_active.is_empty();

    Ok(StartupRecoveryReport {
        is_clean,
        interrupted_sessions: dangling_active,
        incomplete_actions,
    })
}

/// Actively reconciles crashed or interrupted sessions against physical filesystem state.
///
/// Dual Journal-Filesystem Reconciliation Flow:
/// 1. Finds all dangling in-flight actions (`state IN ('Validating', 'Executing')`).
/// 2. For each action, inspects the physical disk at both `quarantine_path` and `source_path`:
///    - If the file exists at `quarantine_path` with matching size and cryptographic BLAKE3 content
///      hash: the atomic move succeeded on disk before the crash; mark `Completed`.
///    - If the file remains at `source_path` with matching size and content hash: the move never
///      occurred before the interruption; mark `Failed`.
///    - If neither exists or content is corrupt: mark `Failed`.
/// 3. Transition dangling `Active` sessions to `Interrupted`.
/// 4. Record chronological diagnostic entries in the SQLite `action_events` audit trail.
pub fn reconcile_startup_state(
    journal: &mut ActionJournal,
) -> Result<RecoveryReport, StorageError> {
    let incomplete_actions = journal.get_incomplete_actions()?;
    let active_or_interrupted = journal.find_active_or_interrupted_sessions()?;

    let dangling_active: Vec<SessionDetails> = active_or_interrupted
        .into_iter()
        .filter(|s| s.status == SessionStatus::Active)
        .collect();

    if incomplete_actions.is_empty() && dangling_active.is_empty() {
        return Ok(RecoveryReport {
            is_clean: true,
            actions_checked: 0,
            actions_resolved_completed: 0,
            actions_resolved_failed: 0,
            sessions_interrupted: 0,
            details: Vec::new(),
        });
    }

    // 1. Mark dangling Active sessions as Interrupted
    let sessions_interrupted = dangling_active.len();
    for session in &dangling_active {
        journal.interrupt_session(session.session_id)?;
    }

    // 2. Perform physical filesystem dual inspection for all incomplete actions
    let mut details = Vec::new();
    let mut actions_resolved_completed = 0;
    let mut actions_resolved_failed = 0;

    for action in incomplete_actions {
        let (final_state, resolution, note) = inspect_and_reconcile_action_filesystem(&action);

        match final_state {
            ActionState::Completed => {
                if action.state == ActionState::Validating {
                    journal.record_action_transition(
                        action.action_id,
                        ActionState::Executing,
                        Some("Recovered transition during crash reconciliation"),
                    )?;
                }
                journal.record_action_transition(
                    action.action_id,
                    ActionState::Completed,
                    Some(&note),
                )?;
                actions_resolved_completed += 1;
            }
            ActionState::Failed => {
                journal.record_action_transition(
                    action.action_id,
                    ActionState::Failed,
                    Some(&note),
                )?;
                actions_resolved_failed += 1;
            }
            _ => {}
        }

        details.push(ActionRecoveryDetail {
            action_id: action.action_id,
            plan_id: action.plan_id,
            source_path: action.source_path,
            quarantine_path: action.quarantine_path,
            initial_state: action.state,
            final_state,
            resolution,
            note,
        });
    }

    Ok(RecoveryReport {
        is_clean: false,
        actions_checked: details.len(),
        actions_resolved_completed,
        actions_resolved_failed,
        sessions_interrupted,
        details,
    })
}

/// Inspects the physical disk state at both source and quarantine paths to determine resolution.
fn inspect_and_reconcile_action_filesystem(
    action: &JournaledAction,
) -> (ActionState, ActionRecoveryResolution, String) {
    let quarantine_meta = fs::symlink_metadata(&action.quarantine_path).ok();
    let quarantine_matches = if let Some(ref meta) = quarantine_meta {
        if meta.file_type().is_file() && meta.len() == action.preconditions.expected_size {
            match compute_file_blake3_hash(&action.quarantine_path) {
                Ok(h) => h == action.preconditions.expected_hash,
                Err(_) => false,
            }
        } else {
            false
        }
    } else {
        false
    };

    let source_meta = fs::symlink_metadata(&action.source_path).ok();
    let source_matches = if let Some(ref meta) = source_meta {
        if meta.file_type().is_file() && meta.len() == action.preconditions.expected_size {
            match compute_file_blake3_hash(&action.source_path) {
                Ok(h) => h == action.preconditions.expected_hash,
                Err(_) => false,
            }
        } else {
            false
        }
    } else {
        false
    };

    if quarantine_matches {
        // Move succeeded before crash; verified intact in quarantine
        let note = "Crash recovery: verified atomic move completed on disk; file verified at quarantine destination with matching hash; marked Completed".to_string();
        (
            ActionState::Completed,
            ActionRecoveryResolution::CompletedAtQuarantine,
            note,
        )
    } else if source_matches {
        // Move never occurred before crash; file intact at source
        let note = "Crash recovery: physical file remains at source path with matching hash; move never executed before interruption; marked Failed".to_string();
        (
            ActionState::Failed,
            ActionRecoveryResolution::UnmovedAtSource,
            note,
        )
    } else {
        // File is missing or corrupted at both paths
        let reason = if quarantine_meta.is_some() {
            "Quarantined file present but size or content hash does not match preconditions"
                .to_string()
        } else if source_meta.is_some() {
            "Source file present but size or content hash does not match preconditions".to_string()
        } else {
            "destination verification required: file missing at both source and quarantine destinations".to_string()
        };

        let note = format!("Crash recovery: {}; marked Failed", reason);
        (
            ActionState::Failed,
            ActionRecoveryResolution::CorruptOrMissing { reason },
            note,
        )
    }
}

/// Reconciles dangling active sessions after an ungraceful crash or process termination.
///
/// Performs full active filesystem recovery and returns a `StartupRecoveryReport`
/// for backwards compatibility.
pub fn reconcile_interrupted_sessions(
    journal: &mut ActionJournal,
) -> Result<StartupRecoveryReport, StorageError> {
    let _ = reconcile_startup_state(journal)?;
    let incomplete = journal.get_incomplete_actions()?;
    let active_or_interrupted = journal.find_active_or_interrupted_sessions()?;
    let dangling_active: Vec<SessionDetails> = active_or_interrupted
        .into_iter()
        .filter(|s| s.status == SessionStatus::Active)
        .collect();

    let is_clean = incomplete.is_empty() && dangling_active.is_empty();

    Ok(StartupRecoveryReport {
        is_clean,
        interrupted_sessions: dangling_active,
        incomplete_actions: incomplete,
    })
}

/// Helper returning the count of dangling active sessions.
pub fn count_dangling_sessions(journal: &ActionJournal) -> Result<usize, StorageError> {
    let report = inspect_startup_state(journal)?;
    Ok(report.interrupted_sessions.len())
}
