//! Error types for the Reclaim storage and journal engine.

use thiserror::Error;

/// Storage errors that can occur during journal operations.
#[derive(Error, Debug)]
pub enum StorageError {
    /// An underlying SQLite error occurred.
    #[error("SQLite database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A JSON serialization or deserialization error occurred.
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    /// The specified session could not be found in the journal.
    #[error("Session '{0}' not found in journal")]
    SessionNotFound(String),

    /// The specified plan could not be found in the journal.
    #[error("Plan '{0}' not found in journal")]
    PlanNotFound(String),

    /// The specified action could not be found in the journal.
    #[error("Action '{0}' not found in journal")]
    ActionNotFound(String),

    /// An invalid action state transition was attempted.
    #[error("Invalid state transition for action '{action_id}': cannot transition from {current} to {target}")]
    InvalidStateTransition {
        /// The affected action ID.
        action_id: String,
        /// The current state of the action.
        current: String,
        /// The attempted target state.
        target: String,
    },

    /// An invalid session state transition was attempted.
    #[error("Invalid state transition for session '{session_id}': cannot transition from {current} to {target}")]
    InvalidSessionStateTransition {
        /// The affected session ID.
        session_id: String,
        /// The current status of the session.
        current: String,
        /// The attempted target status.
        target: String,
    },

    /// A journal record was corrupt or structurally invalid.
    #[error("Corrupt journal record: {0}")]
    CorruptRecord(String),

    /// An IO error occurred while accessing the journal file.
    #[error("Filesystem IO error: {0}")]
    Io(#[from] std::io::Error),
}
