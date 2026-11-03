//! Relational schema definitions, PRAGMAs, and migrations for SQLite storage.

use crate::datetime::format_rfc3339;
use crate::error::StorageError;
use rusqlite::Connection;
use std::time::SystemTime;

/// Current schema version.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// Configures recommended SQLite PRAGMAs for crash resilience and concurrency.
///
/// - `journal_mode = WAL`: Write-Ahead Logging allows concurrent readers and writers.
/// - `foreign_keys = ON`: Enforces referential integrity.
/// - `synchronous = NORMAL`: Flushes critical pages at checkpoints without slowing write transactions.
/// - `busy_timeout = 5000`: Waits up to 5 seconds before returning SQLITE_BUSY.
pub fn apply_pragmas(conn: &Connection) -> Result<(), StorageError> {
    // Foreign keys must be enabled per-connection
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 5000;
        PRAGMA synchronous = NORMAL;
        "#,
    )?;

    // Attempt to set WAL journal mode (may be ignored on in-memory databases)
    let _ = conn.query_row("PRAGMA journal_mode = WAL;", [], |_| Ok(()));

    Ok(())
}

/// Applies schema migrations to bring the database up to `CURRENT_SCHEMA_VERSION`.
pub fn apply_migrations(conn: &mut Connection) -> Result<(), StorageError> {
    apply_pragmas(conn)?;

    let tx = conn.transaction()?;

    // Create schema version tracking table if it doesn't exist
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );
        "#,
    )?;

    let current_version: Option<i64> = tx
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .unwrap_or(None);

    let mut version = current_version.unwrap_or(0);

    if version < 1 {
        tx.execute_batch(
            r#"
            -- Sessions tracking table
            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                target_root TEXT NOT NULL,
                keeper_policy TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('Active', 'Completed', 'Interrupted')),
                started_at TEXT NOT NULL,
                ended_at TEXT
            );

            -- Persisted cleanup plans
            CREATE TABLE IF NOT EXISTS plans (
                plan_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(session_id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                quarantine_root TEXT NOT NULL,
                total_duplicate_files INTEGER NOT NULL,
                duplicate_groups_count INTEGER NOT NULL,
                planned_actions INTEGER NOT NULL,
                protected_keepers INTEGER NOT NULL,
                total_reclaimable_bytes INTEGER NOT NULL,
                files_left_untouched INTEGER NOT NULL,
                plan_json TEXT
            );

            -- Preserved keepers for duplicate groups
            CREATE TABLE IF NOT EXISTS keepers (
                keeper_id TEXT PRIMARY KEY,
                plan_id TEXT NOT NULL REFERENCES plans(plan_id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                size INTEGER NOT NULL,
                hash TEXT NOT NULL,
                file_id TEXT,
                modified TEXT,
                hard_link_aliases_json TEXT NOT NULL,
                rationale TEXT NOT NULL
            );

            -- Proposed and executed actions with state machine tracking
            CREATE TABLE IF NOT EXISTS actions (
                action_id TEXT PRIMARY KEY,
                plan_id TEXT NOT NULL REFERENCES plans(plan_id) ON DELETE CASCADE,
                action_index INTEGER NOT NULL,
                source_path TEXT NOT NULL,
                quarantine_path TEXT NOT NULL,
                keeper_path TEXT NOT NULL,
                rationale TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('Planned', 'Validating', 'Executing', 'Completed', 'Failed', 'Reverted')),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            -- Immutable preconditions recorded at plan time
            CREATE TABLE IF NOT EXISTS preconditions (
                action_id TEXT PRIMARY KEY REFERENCES actions(action_id) ON DELETE CASCADE,
                expected_size INTEGER NOT NULL,
                expected_hash TEXT NOT NULL,
                source_modified TEXT,
                source_file_id TEXT,
                keeper_modified TEXT,
                keeper_file_id TEXT
            );

            -- Chronological audit trail for action state transitions
            CREATE TABLE IF NOT EXISTS action_events (
                event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                action_id TEXT NOT NULL REFERENCES actions(action_id) ON DELETE CASCADE,
                from_state TEXT,
                to_state TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                details TEXT
            );

            -- Indices for fast querying and crash-recovery scans
            CREATE INDEX IF NOT EXISTS idx_plans_session ON plans(session_id);
            CREATE INDEX IF NOT EXISTS idx_keepers_plan ON keepers(plan_id);
            CREATE INDEX IF NOT EXISTS idx_actions_plan ON actions(plan_id);
            CREATE INDEX IF NOT EXISTS idx_actions_state ON actions(state);
            CREATE INDEX IF NOT EXISTS idx_events_action ON action_events(action_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
            "#,
        )?;

        let now = format_rfc3339(SystemTime::now());
        tx.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (1, ?)",
            [now],
        )?;

        version = 1;
    }

    let _ = version;
    tx.commit()?;
    Ok(())
}
