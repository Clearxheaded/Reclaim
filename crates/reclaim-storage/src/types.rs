//! Domain models, strong identifiers, and state machine definitions for the journal.

use crate::error::StorageError;
use reclaim_core::planner::{ActionPreconditions, PlanSummary, ProtectedKeeper};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;
use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;

macro_rules! define_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generates a new random UUID v4 identifier.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an identifier from an existing UUID.
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Parses an identifier from a UUID string.
            pub fn parse_str(s: &str) -> Result<Self, uuid::Error> {
                Ok(Self(Uuid::parse_str(s)?))
            }

            /// Returns the inner UUID.
            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl Deref for $name {
            type Target = Uuid;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}

define_id!(
    SessionId,
    "Strongly typed identifier for a Reclaim session."
);
define_id!(
    PlanId,
    "Strongly typed identifier for a persisted CleanupPlan."
);
define_id!(
    ActionId,
    "Strongly typed identifier for a persisted journal action."
);

/// The operational status of a Reclaim execution session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionStatus {
    /// The session is currently active and in progress.
    Active,
    /// The session completed normally without interruption.
    Completed,
    /// The session was interrupted by an unexpected crash, cancellation, or error.
    Interrupted,
}

impl SessionStatus {
    /// String representation for SQLite storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Active => "Active",
            SessionStatus::Completed => "Completed",
            SessionStatus::Interrupted => "Interrupted",
        }
    }
}

impl std::str::FromStr for SessionStatus {
    type Err = StorageError;

    /// Parses status from a stored SQLite string.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Active" => Ok(SessionStatus::Active),
            "Completed" => Ok(SessionStatus::Completed),
            "Interrupted" => Ok(SessionStatus::Interrupted),
            other => Err(StorageError::CorruptRecord(format!(
                "Unknown session status: '{other}'"
            ))),
        }
    }
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// The lifecycle state of a quarantine action in the durable journal.
///
/// State machine flow:
/// ```text
/// Planned ---> Validating ---> Executing ---> Completed ---> Reverted (Undo)
///                 |               |
///                 v               v
///               Failed          Failed
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionState {
    /// Prospective action recorded from plan; no filesystem checks or moves initiated.
    Planned,
    /// Preconditions (hashes, identities, sizes) are actively being revalidated (TOCTOU defense).
    Validating,
    /// Preconditions passed; atomic quarantine move is actively in-flight.
    Executing,
    /// Action completed and committed successfully.
    Completed,
    /// Action failed (precondition mismatch, destination conflict, or IO error).
    Failed,
    /// Action was reverted/restored during an undo operation (Milestone 6).
    Reverted,
}

impl ActionState {
    /// String representation for SQLite storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionState::Planned => "Planned",
            ActionState::Validating => "Validating",
            ActionState::Executing => "Executing",
            ActionState::Completed => "Completed",
            ActionState::Failed => "Failed",
            ActionState::Reverted => "Reverted",
        }
    }

    /// Returns `true` if the action is in an incomplete, in-flight state.
    ///
    /// Used during crash recovery to detect actions that were interrupted mid-execution.
    pub fn is_incomplete(&self) -> bool {
        matches!(self, ActionState::Validating | ActionState::Executing)
    }

    /// Returns `true` if this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ActionState::Completed | ActionState::Failed | ActionState::Reverted
        )
    }

    /// Validates whether a state transition from `self` to `next` is permissible.
    pub fn can_transition_to(&self, next: ActionState) -> bool {
        match (self, next) {
            // Forward progression
            (ActionState::Planned, ActionState::Validating) => true,
            (ActionState::Validating, ActionState::Executing) => true,
            (ActionState::Executing, ActionState::Completed) => true,

            // Failure branches
            (ActionState::Planned, ActionState::Failed) => true,
            (ActionState::Validating, ActionState::Failed) => true,
            (ActionState::Executing, ActionState::Failed) => true,

            // Reversion branch (Undo)
            (ActionState::Completed, ActionState::Reverted) => true,

            // Disallow all other transitions
            _ => false,
        }
    }
}

impl std::str::FromStr for ActionState {
    type Err = StorageError;

    /// Parses an action state from a stored SQLite string.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Planned" => Ok(ActionState::Planned),
            "Validating" => Ok(ActionState::Validating),
            "Executing" => Ok(ActionState::Executing),
            "Completed" => Ok(ActionState::Completed),
            "Failed" => Ok(ActionState::Failed),
            "Reverted" => Ok(ActionState::Reverted),
            other => Err(StorageError::CorruptRecord(format!(
                "Unknown action state: '{other}'"
            ))),
        }
    }
}

impl fmt::Display for ActionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A persisted action with its durable identifier, state, and preconditions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournaledAction {
    /// Unique identifier for this action in the journal.
    pub action_id: ActionId,
    /// Reference to the plan this action belongs to.
    pub plan_id: PlanId,
    /// Zero-based sequence index within the plan.
    pub action_index: usize,
    /// Path of the source duplicate file to be quarantined.
    pub source_path: PathBuf,
    /// Staged destination path in quarantine.
    pub quarantine_path: PathBuf,
    /// Path of the verified keeper file preserving this content.
    pub keeper_path: PathBuf,
    /// Human-readable explanation of why this action was proposed.
    pub rationale: String,
    /// Current state in the action state machine.
    pub state: ActionState,
    /// Frozen preconditions required for safe execution.
    pub preconditions: ActionPreconditions,
    /// Creation timestamp.
    pub created_at: SystemTime,
    /// Last update timestamp.
    pub updated_at: SystemTime,
}

/// An entry in the chronological audit log recording an action state transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEvent {
    /// Monotonically increasing event ID.
    pub event_id: i64,
    /// The action ID associated with this event.
    pub action_id: ActionId,
    /// Previous state, or `None` on initial plan registration.
    pub from_state: Option<ActionState>,
    /// New state transitioned into.
    pub to_state: ActionState,
    /// Timestamp when this transition was committed.
    pub timestamp: SystemTime,
    /// Optional diagnostic message, error report, or rationale.
    pub details: Option<String>,
}

/// Detailed historical record of a Reclaim session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDetails {
    /// Session identifier.
    pub session_id: SessionId,
    /// Root directory targeted by this session.
    pub target_root: PathBuf,
    /// Serialized keeper policy used.
    pub keeper_policy: String,
    /// Current session status.
    pub status: SessionStatus,
    /// Start timestamp.
    pub started_at: SystemTime,
    /// Completion or interruption timestamp.
    pub ended_at: Option<SystemTime>,
    /// IDs of all cleanup plans generated under this session.
    pub plan_ids: Vec<PlanId>,
}

/// Detailed record of a persisted cleanup plan including all actions and keepers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDetails {
    /// Plan identifier.
    pub plan_id: PlanId,
    /// Reference to parent session.
    pub session_id: SessionId,
    /// Plan creation timestamp.
    pub created_at: SystemTime,
    /// Target quarantine staging directory.
    pub quarantine_root: PathBuf,
    /// High-level metrics summarizing the plan.
    pub summary: PlanSummary,
    /// All actions in this plan.
    pub actions: Vec<JournaledAction>,
    /// All protected keepers preserved in this plan.
    pub keepers: Vec<ProtectedKeeper>,
}

/// Aggregated historical summary of an execution session for audit and CLI reporting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistorySummary {
    /// Session identifier.
    pub session_id: SessionId,
    /// Root directory targeted by this session.
    pub target_root: PathBuf,
    /// Current session status.
    pub status: SessionStatus,
    /// Start timestamp.
    pub started_at: SystemTime,
    /// Completion or interruption timestamp.
    pub ended_at: Option<SystemTime>,
    /// Total plans generated under this session.
    pub total_plans: usize,
    /// Total actions recorded across all plans in this session.
    pub total_actions: usize,
    /// Total actions completed (currently staged in quarantine).
    pub completed_actions: usize,
    /// Total actions reverted (safely restored from quarantine).
    pub reverted_actions: usize,
    /// Total actions that failed during validation or mutation.
    pub failed_actions: usize,
    /// Active bytes currently held in quarantine for this session.
    pub active_quarantined_bytes: u64,
    /// Total bytes restored from quarantine back to original locations.
    pub restored_bytes: u64,
}
