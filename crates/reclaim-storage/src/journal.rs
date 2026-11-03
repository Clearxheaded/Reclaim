//! Durable SQLite-backed action journal and transaction manager.

use crate::datetime::{format_rfc3339, parse_rfc3339};
use crate::error::StorageError;
use crate::schema::{apply_migrations, apply_pragmas};
use crate::types::{
    ActionId, ActionState, JournalEvent, JournaledAction, PlanDetails, PlanId, SessionDetails,
    SessionId, SessionStatus,
};
use reclaim_core::file_id::FileId;
use reclaim_core::planner::{
    ActionPreconditions, CleanupPlan, KeeperPolicy, PlanSummary, ProtectedKeeper,
};
use rusqlite::{params, Connection, Row};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::SystemTime;
use uuid::Uuid;

/// Durable action journal managing sessions, plans, actions, and audit logs in SQLite.
pub struct ActionJournal {
    conn: Connection,
    active_session_id: Option<SessionId>,
}

impl ActionJournal {
    /// Opens or creates a durable journal at the given filesystem path.
    ///
    /// Configures WAL mode, synchronous=NORMAL, foreign keys, and executes migrations.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut conn = Connection::open(path)?;
        apply_migrations(&mut conn)?;

        Ok(Self {
            conn,
            active_session_id: None,
        })
    }

    /// Opens an in-memory journal, ideal for unit testing and ephemeral sessions.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let mut conn = Connection::open_in_memory()?;
        apply_migrations(&mut conn)?;

        Ok(Self {
            conn,
            active_session_id: None,
        })
    }

    /// Wraps an existing SQLite connection, ensuring PRAGMAs and schema migrations are applied.
    pub fn new_with_connection(mut conn: Connection) -> Result<Self, StorageError> {
        apply_pragmas(&conn)?;
        apply_migrations(&mut conn)?;

        Ok(Self {
            conn,
            active_session_id: None,
        })
    }

    /// Returns the currently active session ID, if any has been created on this instance.
    pub fn active_session_id(&self) -> Option<SessionId> {
        self.active_session_id
    }

    /// Sets the active session ID.
    pub fn set_active_session_id(&mut self, session_id: Option<SessionId>) {
        self.active_session_id = session_id;
    }

    /// Creates a new execution session in the journal with status `Active`.
    pub fn create_session(
        &mut self,
        target_root: &Path,
        policy: &KeeperPolicy,
    ) -> Result<SessionId, StorageError> {
        let session_id = SessionId::new();
        let policy_json = serde_json::to_string(policy)?;
        let started_at = format_rfc3339(SystemTime::now());

        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO sessions (session_id, target_root, keeper_policy, status, started_at, ended_at)
            VALUES (?, ?, ?, 'Active', ?, NULL)
            "#,
            params![
                session_id.to_string(),
                target_root.to_string_lossy(),
                policy_json,
                started_at,
            ],
        )?;
        tx.commit()?;

        self.active_session_id = Some(session_id);
        Ok(session_id)
    }

    /// Marks an active session as `Completed`.
    pub fn complete_session(&mut self, session_id: SessionId) -> Result<(), StorageError> {
        let tx = self.conn.transaction()?;

        let current_status: String = tx
            .query_row(
                "SELECT status FROM sessions WHERE session_id = ?",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StorageError::SessionNotFound(session_id.to_string())
                }
                other => StorageError::Sqlite(other),
            })?;

        if current_status != SessionStatus::Active.as_str() {
            return Err(StorageError::InvalidSessionStateTransition {
                session_id: session_id.to_string(),
                current: current_status,
                target: SessionStatus::Completed.to_string(),
            });
        }

        let ended_at = format_rfc3339(SystemTime::now());
        tx.execute(
            "UPDATE sessions SET status = 'Completed', ended_at = ? WHERE session_id = ?",
            params![ended_at, session_id.to_string()],
        )?;

        tx.commit()?;
        if self.active_session_id == Some(session_id) {
            self.active_session_id = None;
        }
        Ok(())
    }

    /// Marks an active session as `Interrupted`.
    pub fn interrupt_session(&mut self, session_id: SessionId) -> Result<(), StorageError> {
        let tx = self.conn.transaction()?;

        let current_status: String = tx
            .query_row(
                "SELECT status FROM sessions WHERE session_id = ?",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StorageError::SessionNotFound(session_id.to_string())
                }
                other => StorageError::Sqlite(other),
            })?;

        if current_status == SessionStatus::Completed.as_str() {
            return Err(StorageError::InvalidSessionStateTransition {
                session_id: session_id.to_string(),
                current: current_status,
                target: SessionStatus::Interrupted.to_string(),
            });
        }

        let ended_at = format_rfc3339(SystemTime::now());
        tx.execute(
            "UPDATE sessions SET status = 'Interrupted', ended_at = ? WHERE session_id = ?",
            params![ended_at, session_id.to_string()],
        )?;

        tx.commit()?;
        if self.active_session_id == Some(session_id) {
            self.active_session_id = None;
        }
        Ok(())
    }

    /// Persists a prospective cleanup plan, linking it to the active session or creating one.
    pub fn save_plan(&mut self, plan: &CleanupPlan) -> Result<PlanId, StorageError> {
        let session_id = match self.active_session_id {
            Some(id) => id,
            None => self.create_session(&plan.root_dir, &plan.policy)?,
        };

        self.save_plan_for_session(session_id, plan)
    }

    /// Persists a prospective cleanup plan under a specified session.
    ///
    /// Executes within an explicit SQLite transaction: if any action, precondition,
    /// or keeper fails to persist, all changes are rolled back automatically.
    pub fn save_plan_for_session(
        &mut self,
        session_id: SessionId,
        plan: &CleanupPlan,
    ) -> Result<PlanId, StorageError> {
        let plan_id = PlanId::new();
        let created_at_str = format_rfc3339(plan.created_at);
        let plan_json = serde_json::to_string(plan)?;

        let tx = self.conn.transaction()?;

        // Verify session existence
        let session_exists: bool = tx
            .query_row(
                "SELECT 1 FROM sessions WHERE session_id = ?",
                [session_id.to_string()],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !session_exists {
            return Err(StorageError::SessionNotFound(session_id.to_string()));
        }

        // 1. Insert plan row
        tx.execute(
            r#"
            INSERT INTO plans (
                plan_id, session_id, created_at, quarantine_root,
                total_duplicate_files, duplicate_groups_count,
                planned_actions, protected_keepers,
                total_reclaimable_bytes, files_left_untouched, plan_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            params![
                plan_id.to_string(),
                session_id.to_string(),
                created_at_str,
                plan.quarantine_dir.to_string_lossy(),
                plan.summary.total_duplicate_files as i64,
                plan.summary.duplicate_groups_count as i64,
                plan.summary.planned_actions as i64,
                plan.summary.protected_keepers as i64,
                plan.summary.total_reclaimable_bytes as i64,
                plan.summary.files_left_untouched as i64,
                plan_json,
            ],
        )?;

        // 2. Insert keepers
        for keeper in &plan.keepers {
            let keeper_id = Uuid::new_v4().to_string();
            let file_id_json = keeper
                .file_id
                .map(|fid| serde_json::to_string(&fid))
                .transpose()?;
            let modified_str = keeper.modified.map(format_rfc3339);
            let aliases_json = serde_json::to_string(&keeper.hard_link_aliases)?;

            tx.execute(
                r#"
                INSERT INTO keepers (
                    keeper_id, plan_id, path, size, hash, file_id, modified, hard_link_aliases_json, rationale
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
                params![
                    keeper_id,
                    plan_id.to_string(),
                    keeper.path.to_string_lossy(),
                    keeper.size as i64,
                    keeper.hash,
                    file_id_json,
                    modified_str,
                    aliases_json,
                    keeper.rationale,
                ],
            )?;
        }

        // 3. Insert actions, preconditions, and initial audit trail events
        for (idx, action) in plan.actions.iter().enumerate() {
            let action_id = ActionId::new();

            tx.execute(
                r#"
                INSERT INTO actions (
                    action_id, plan_id, action_index, source_path,
                    quarantine_path, keeper_path, rationale, state,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, 'Planned', ?, ?)
                "#,
                params![
                    action_id.to_string(),
                    plan_id.to_string(),
                    idx as i64,
                    action.source_path.to_string_lossy(),
                    action.quarantine_path.to_string_lossy(),
                    action.keeper_path.to_string_lossy(),
                    action.rationale,
                    created_at_str,
                    created_at_str,
                ],
            )?;

            let source_fid_json = action
                .preconditions
                .source_file_id
                .map(|fid| serde_json::to_string(&fid))
                .transpose()?;
            let keeper_fid_json = action
                .preconditions
                .keeper_file_id
                .map(|fid| serde_json::to_string(&fid))
                .transpose()?;
            let source_mod_str = action.preconditions.source_modified.map(format_rfc3339);
            let keeper_mod_str = action.preconditions.keeper_modified.map(format_rfc3339);

            tx.execute(
                r#"
                INSERT INTO preconditions (
                    action_id, expected_size, expected_hash,
                    source_modified, source_file_id,
                    keeper_modified, keeper_file_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
                params![
                    action_id.to_string(),
                    action.preconditions.expected_size as i64,
                    action.preconditions.expected_hash,
                    source_mod_str,
                    source_fid_json,
                    keeper_mod_str,
                    keeper_fid_json,
                ],
            )?;

            // Initial audit log event
            tx.execute(
                r#"
                INSERT INTO action_events (
                    action_id, from_state, to_state, timestamp, details
                ) VALUES (?, NULL, 'Planned', ?, 'Action registered from cleanup plan')
                "#,
                params![action_id.to_string(), created_at_str,],
            )?;
        }

        tx.commit()?;
        Ok(plan_id)
    }

    /// Records an action state transition, enforcing the action state machine and logging an audit event.
    pub fn record_action_transition(
        &mut self,
        action_id: ActionId,
        new_state: ActionState,
        details: Option<&str>,
    ) -> Result<(), StorageError> {
        let tx = self.conn.transaction()?;

        let current_state_str: String = tx
            .query_row(
                "SELECT state FROM actions WHERE action_id = ?",
                [action_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StorageError::ActionNotFound(action_id.to_string())
                }
                other => StorageError::Sqlite(other),
            })?;

        let current_state = ActionState::from_str(&current_state_str)?;

        if !current_state.can_transition_to(new_state) {
            return Err(StorageError::InvalidStateTransition {
                action_id: action_id.to_string(),
                current: current_state.to_string(),
                target: new_state.to_string(),
            });
        }

        let now = format_rfc3339(SystemTime::now());

        tx.execute(
            "UPDATE actions SET state = ?, updated_at = ? WHERE action_id = ?",
            params![new_state.as_str(), now, action_id.to_string()],
        )?;

        tx.execute(
            r#"
            INSERT INTO action_events (action_id, from_state, to_state, timestamp, details)
            VALUES (?, ?, ?, ?, ?)
            "#,
            params![
                action_id.to_string(),
                current_state.as_str(),
                new_state.as_str(),
                now,
                details,
            ],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Returns all actions that were left in an in-flight state (`Validating` or `Executing`).
    ///
    /// This is the primary recovery query called after an unexpected crash or kill.
    pub fn get_incomplete_actions(&self) -> Result<Vec<JournaledAction>, StorageError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                a.action_id, a.plan_id, a.action_index, a.source_path, a.quarantine_path, a.keeper_path,
                a.rationale, a.state, a.created_at, a.updated_at,
                p.expected_size, p.expected_hash, p.source_modified, p.source_file_id, p.keeper_modified, p.keeper_file_id
            FROM actions a
            JOIN preconditions p ON a.action_id = p.action_id
            WHERE a.state IN ('Validating', 'Executing')
            ORDER BY a.action_index ASC
            "#,
        )?;

        let rows = stmt.query_map([], parse_journaled_action_row)?;
        let mut actions = Vec::new();
        for row in rows {
            actions.push(row?);
        }

        Ok(actions)
    }

    /// Fetches an action by its unique `ActionId`.
    pub fn get_action(&self, action_id: ActionId) -> Result<JournaledAction, StorageError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                a.action_id, a.plan_id, a.action_index, a.source_path, a.quarantine_path, a.keeper_path,
                a.rationale, a.state, a.created_at, a.updated_at,
                p.expected_size, p.expected_hash, p.source_modified, p.source_file_id, p.keeper_modified, p.keeper_file_id
            FROM actions a
            JOIN preconditions p ON a.action_id = p.action_id
            WHERE a.action_id = ?
            "#,
        )?;

        stmt.query_row([action_id.to_string()], parse_journaled_action_row)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StorageError::ActionNotFound(action_id.to_string())
                }
                other => StorageError::Sqlite(other),
            })
    }

    /// Lists all actions belonging to a plan in index order.
    pub fn list_actions_for_plan(
        &self,
        plan_id: PlanId,
    ) -> Result<Vec<JournaledAction>, StorageError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                a.action_id, a.plan_id, a.action_index, a.source_path, a.quarantine_path, a.keeper_path,
                a.rationale, a.state, a.created_at, a.updated_at,
                p.expected_size, p.expected_hash, p.source_modified, p.source_file_id, p.keeper_modified, p.keeper_file_id
            FROM actions a
            JOIN preconditions p ON a.action_id = p.action_id
            WHERE a.plan_id = ?
            ORDER BY a.action_index ASC
            "#,
        )?;

        let rows = stmt.query_map([plan_id.to_string()], parse_journaled_action_row)?;
        let mut actions = Vec::new();
        for row in rows {
            actions.push(row?);
        }

        Ok(actions)
    }

    /// Fetches the chronological audit trail of state transitions for an action.
    pub fn fetch_action_audit_trail(
        &self,
        action_id: ActionId,
    ) -> Result<Vec<JournalEvent>, StorageError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT event_id, action_id, from_state, to_state, timestamp, details
            FROM action_events
            WHERE action_id = ?
            ORDER BY event_id ASC
            "#,
        )?;

        let rows = stmt.query_map([action_id.to_string()], |row| {
            let event_id: i64 = row.get(0)?;
            let action_id_str: String = row.get(1)?;
            let from_state_str: Option<String> = row.get(2)?;
            let to_state_str: String = row.get(3)?;
            let timestamp_str: String = row.get(4)?;
            let details: Option<String> = row.get(5)?;

            let action_id = ActionId::parse_str(&action_id_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let from_state = from_state_str
                .map(|s| {
                    ActionState::from_str(&s)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
                })
                .transpose()?;
            let to_state = ActionState::from_str(&to_state_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let timestamp = parse_rfc3339(&timestamp_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            Ok(JournalEvent {
                event_id,
                action_id,
                from_state,
                to_state,
                timestamp,
                details,
            })
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }

        Ok(events)
    }

    /// Fetches complete history for a session, including its associated plan IDs.
    pub fn fetch_session_history(
        &self,
        session_id: SessionId,
    ) -> Result<SessionDetails, StorageError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT session_id, target_root, keeper_policy, status, started_at, ended_at
            FROM sessions
            WHERE session_id = ?
            "#,
        )?;

        let session = stmt
            .query_row([session_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let root_str: String = row.get(1)?;
                let policy_str: String = row.get(2)?;
                let status_str: String = row.get(3)?;
                let started_str: String = row.get(4)?;
                let ended_str: Option<String> = row.get(5)?;

                let session_id = SessionId::parse_str(&id_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let status = SessionStatus::from_str(&status_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let started_at = parse_rfc3339(&started_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let ended_at = ended_str
                    .map(|s| {
                        parse_rfc3339(&s)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
                    })
                    .transpose()?;

                Ok(SessionDetails {
                    session_id,
                    target_root: PathBuf::from(root_str),
                    keeper_policy: policy_str,
                    status,
                    started_at,
                    ended_at,
                    plan_ids: Vec::new(),
                })
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StorageError::SessionNotFound(session_id.to_string())
                }
                other => StorageError::Sqlite(other),
            })?;

        // Retrieve plan IDs associated with this session
        let mut plan_stmt = self
            .conn
            .prepare("SELECT plan_id FROM plans WHERE session_id = ? ORDER BY created_at ASC")?;
        let plan_rows = plan_stmt.query_map([session_id.to_string()], |row| {
            let pid_str: String = row.get(0)?;
            let plan_id = PlanId::parse_str(&pid_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(plan_id)
        })?;

        let mut plan_ids = Vec::new();
        for pid in plan_rows {
            plan_ids.push(pid?);
        }

        let mut result = session;
        result.plan_ids = plan_ids;
        Ok(result)
    }

    /// Fetches full plan details including summary, actions, and protected keepers.
    pub fn fetch_plan(&self, plan_id: PlanId) -> Result<PlanDetails, StorageError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT plan_id, session_id, created_at, quarantine_root,
                   total_duplicate_files, duplicate_groups_count, planned_actions,
                   protected_keepers, total_reclaimable_bytes, files_left_untouched
            FROM plans
            WHERE plan_id = ?
            "#,
        )?;

        let (session_id, created_at, quarantine_root, summary) = stmt
            .query_row([plan_id.to_string()], |row| {
                let _pid_str: String = row.get(0)?;
                let sid_str: String = row.get(1)?;
                let created_str: String = row.get(2)?;
                let qroot_str: String = row.get(3)?;
                let total_dup: i64 = row.get(4)?;
                let dup_groups: i64 = row.get(5)?;
                let actions_cnt: i64 = row.get(6)?;
                let keepers_cnt: i64 = row.get(7)?;
                let bytes: i64 = row.get(8)?;
                let untouched: i64 = row.get(9)?;

                let session_id = SessionId::parse_str(&sid_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let created_at = parse_rfc3339(&created_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                let summary = PlanSummary {
                    total_duplicate_files: total_dup as usize,
                    duplicate_groups_count: dup_groups as usize,
                    planned_actions: actions_cnt as usize,
                    protected_keepers: keepers_cnt as usize,
                    total_reclaimable_bytes: bytes as u64,
                    files_left_untouched: untouched as usize,
                };

                Ok((session_id, created_at, PathBuf::from(qroot_str), summary))
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StorageError::PlanNotFound(plan_id.to_string())
                }
                other => StorageError::Sqlite(other),
            })?;

        let actions = self.list_actions_for_plan(plan_id)?;

        // Fetch keepers
        let mut keeper_stmt = self.conn.prepare(
            r#"
            SELECT path, size, hash, file_id, modified, hard_link_aliases_json, rationale
            FROM keepers
            WHERE plan_id = ?
            "#,
        )?;

        let keeper_rows = keeper_stmt.query_map([plan_id.to_string()], |row| {
            let path_str: String = row.get(0)?;
            let size: i64 = row.get(1)?;
            let hash: String = row.get(2)?;
            let file_id_str: Option<String> = row.get(3)?;
            let modified_str: Option<String> = row.get(4)?;
            let aliases_json: String = row.get(5)?;
            let rationale: String = row.get(6)?;

            let file_id: Option<FileId> = file_id_str
                .map(|s| {
                    serde_json::from_str(&s)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
                })
                .transpose()?;

            let modified = modified_str
                .map(|s| {
                    parse_rfc3339(&s)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
                })
                .transpose()?;

            let aliases: Vec<PathBuf> = serde_json::from_str(&aliases_json)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            Ok(ProtectedKeeper {
                path: PathBuf::from(path_str),
                size: size as u64,
                hash,
                file_id,
                modified,
                hard_link_aliases: aliases,
                rationale,
            })
        })?;

        let mut keepers = Vec::new();
        for k in keeper_rows {
            keepers.push(k?);
        }

        Ok(PlanDetails {
            plan_id,
            session_id,
            created_at,
            quarantine_root,
            summary,
            actions,
            keepers,
        })
    }

    /// Finds all sessions that are currently `Active` or `Interrupted`.
    pub fn find_active_or_interrupted_sessions(&self) -> Result<Vec<SessionDetails>, StorageError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT session_id
            FROM sessions
            WHERE status IN ('Active', 'Interrupted')
            ORDER BY started_at DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let sid_str: String = row.get(0)?;
            let session_id = SessionId::parse_str(&sid_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(session_id)
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let sid = row?;
            sessions.push(self.fetch_session_history(sid)?);
        }

        Ok(sessions)
    }

    /// Exposes underlying SQLite connection reference for testing or raw inspection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// Helper mapping an SQLite row join (`actions` + `preconditions`) to a `JournaledAction`.
fn parse_journaled_action_row(row: &Row) -> rusqlite::Result<JournaledAction> {
    let action_id_str: String = row.get(0)?;
    let plan_id_str: String = row.get(1)?;
    let action_index: i64 = row.get(2)?;
    let source_path_str: String = row.get(3)?;
    let quarantine_path_str: String = row.get(4)?;
    let keeper_path_str: String = row.get(5)?;
    let rationale: String = row.get(6)?;
    let state_str: String = row.get(7)?;
    let created_at_str: String = row.get(8)?;
    let updated_at_str: String = row.get(9)?;

    let expected_size: i64 = row.get(10)?;
    let expected_hash: String = row.get(11)?;
    let source_modified_str: Option<String> = row.get(12)?;
    let source_file_id_str: Option<String> = row.get(13)?;
    let keeper_modified_str: Option<String> = row.get(14)?;
    let keeper_file_id_str: Option<String> = row.get(15)?;

    let action_id = ActionId::parse_str(&action_id_str)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let plan_id = PlanId::parse_str(&plan_id_str)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let state = ActionState::from_str(&state_str)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    let created_at = parse_rfc3339(&created_at_str)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let updated_at = parse_rfc3339(&updated_at_str)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    let source_modified = source_modified_str
        .map(|s| {
            parse_rfc3339(&s).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })
        .transpose()?;

    let keeper_modified = keeper_modified_str
        .map(|s| {
            parse_rfc3339(&s).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })
        .transpose()?;

    let source_file_id: Option<FileId> = source_file_id_str
        .map(|s| {
            serde_json::from_str(&s)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })
        .transpose()?;

    let keeper_file_id: Option<FileId> = keeper_file_id_str
        .map(|s| {
            serde_json::from_str(&s)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })
        .transpose()?;

    let preconditions = ActionPreconditions {
        expected_size: expected_size as u64,
        expected_hash,
        source_modified,
        source_file_id,
        keeper_modified,
        keeper_file_id,
    };

    Ok(JournaledAction {
        action_id,
        plan_id,
        action_index: action_index as usize,
        source_path: PathBuf::from(source_path_str),
        quarantine_path: PathBuf::from(quarantine_path_str),
        keeper_path: PathBuf::from(keeper_path_str),
        rationale,
        state,
        preconditions,
        created_at,
        updated_at,
    })
}
