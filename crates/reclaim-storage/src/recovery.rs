//! Crash inspection, recovery assessment, and reconciliation utilities.
//!
//! Evaluates journal state on application startup to detect unclean terminations,
//! dangling in-flight mutations, and interrupted sessions.

use crate::error::StorageError;
use crate::journal::ActionJournal;
use crate::types::{ActionState, JournaledAction, SessionDetails, SessionStatus};

/// Summary of startup journal inspection and recovery status.
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

/// Reconciles dangling active sessions after an ungraceful crash or process termination.
///
/// Marks any `Active` sessions as `Interrupted` with the current timestamp,
/// and logs diagnostic entries in the audit log for any incomplete actions.
pub fn reconcile_interrupted_sessions(
    journal: &mut ActionJournal,
) -> Result<StartupRecoveryReport, StorageError> {
    let report = inspect_startup_state(journal)?;

    if report.is_clean {
        return Ok(report);
    }

    // 1. Mark dangling Active sessions as Interrupted
    for session in &report.interrupted_sessions {
        journal.interrupt_session(session.session_id)?;
    }

    // 2. Add diagnostic audit entries for incomplete actions noting the crash
    for action in &report.incomplete_actions {
        let note = match action.state {
            ActionState::Validating => {
                "Process terminated while validating preconditions; no filesystem mutation occurred"
            }
            ActionState::Executing => {
                "Process terminated during mutation execution; destination verification required"
            }
            _ => "Action was in-flight during unclean termination",
        };

        // Transition incomplete action to Failed to prevent dangling state, noting crash in audit trail
        let _ = journal.record_action_transition(action.action_id, ActionState::Failed, Some(note));
    }

    // Re-inspect to construct final post-reconciliation report
    let updated_incomplete = journal.get_incomplete_actions()?;
    let mut updated_sessions = Vec::new();
    for session in &report.interrupted_sessions {
        if let Ok(updated) = journal.fetch_session_history(session.session_id) {
            updated_sessions.push(updated);
        }
    }

    Ok(StartupRecoveryReport {
        is_clean: updated_incomplete.is_empty(),
        interrupted_sessions: updated_sessions,
        incomplete_actions: updated_incomplete,
    })
}

/// Helper returning the count of dangling active sessions.
pub fn count_dangling_sessions(journal: &ActionJournal) -> Result<usize, StorageError> {
    let report = inspect_startup_state(journal)?;
    Ok(report.interrupted_sessions.len())
}
