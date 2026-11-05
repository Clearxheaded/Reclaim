//! # Milestone 8: Automated Fault-Injection & Reliability Integration Tests
//!
//! Rigorously verifies Reclaim's 10 Filesystem Safety Invariants under adverse, simulated
//! failure modes:
//! 1. Process termination / power loss mid-syscall (before move vs. after move before journal commit).
//! 2. Multi-action batch crashes and subsequent reconciliation + conditional undo.
//! 3. Permission drops (unreadable source, unreadable keeper, unwritable quarantine, unwritable destination on undo).
//! 4. Missing keeper files (race conditions between planning and execution, or execution and undo).
//! 5. Destination collisions (occupied quarantine path, occupied source path on undo).
//! 6. Storage & I/O failure modes (tampered quarantine payload, truncated crash recovery, path blocked by file).

use reclaim_core::duplicates::{find_duplicates, DuplicateOptions};
use reclaim_core::mutation::atomic_move_no_replace;
use reclaim_core::planner::{generate_plan, CleanupPlan, KeeperPolicy, PlanOptions};
use reclaim_core::scanner::{scan_directory, ScanOptions};
use reclaim_storage::recovery::{
    inspect_startup_state, reconcile_startup_state, ActionRecoveryResolution,
};
use reclaim_storage::undo::{SafeReverter, UndoConflictReason, UndoOptions, UndoStatus};
use reclaim_storage::{ActionJournal, ActionState, ExecutionOptions, SafeExecutor};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// RAII guard ensuring Unix file/directory permissions are always restored upon scope exit.
#[cfg(unix)]
struct PermissionGuard {
    path: PathBuf,
    restore_mode: u32,
}

#[cfg(unix)]
impl PermissionGuard {
    fn new(path: &Path, temporary_mode: u32, restore_mode: u32) -> Self {
        fs::set_permissions(path, fs::Permissions::from_mode(temporary_mode)).unwrap();
        Self {
            path: path.to_path_buf(),
            restore_mode,
        }
    }
}

#[cfg(unix)]
impl Drop for PermissionGuard {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.restore_mode));
    }
}

/// Helper building a real directory structure with exact duplicate files.
fn setup_test_files(
    root: &Path,
    count: usize,
    content: &[u8],
) -> (PathBuf, Vec<PathBuf>, CleanupPlan) {
    let canonical_dir = root.join("a");
    let copies_dir = root.join("copies_long_directory");
    fs::create_dir_all(&canonical_dir).unwrap();
    fs::create_dir_all(&copies_dir).unwrap();

    let keeper_file = canonical_dir.join("k.bin");
    fs::write(&keeper_file, content).unwrap();

    let mut dup_files = Vec::with_capacity(count);
    for i in 0..count {
        let dup_file = copies_dir.join(format!("duplicate_file_payload_{}.bin", i));
        fs::write(&dup_file, content).unwrap();
        dup_files.push(dup_file);
    }

    let quarantine_dir = root.join(".reclaim").join("quarantine");

    let scan_opts = ScanOptions::default();
    let scan_out = scan_directory(root, scan_opts).unwrap();

    let dup_opts = DuplicateOptions::default();
    let dup_out = find_duplicates(&scan_out.entries, &dup_opts);

    let plan_opts = PlanOptions {
        root_dir: root.to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: Some(quarantine_dir),
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();
    (keeper_file, dup_files, plan)
}

// =========================================================================
// 1. Process Interruption Mid-Syscall Tests
// =========================================================================

#[test]
fn test_fault_injection_crash_before_filesystem_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content =
        b"Crash simulation: intent recorded in SQLite, process killed before rename syscall";
    let (keeper_path, dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let session_id = journal
        .create_session(&plan.root_dir, &plan.policy)
        .unwrap();
    let plan_id = journal.save_plan_for_session(session_id, &plan).unwrap();
    let plan_details = journal.fetch_plan(plan_id).unwrap();
    let action = &plan_details.actions[0];

    // Transition Planned -> Validating -> Executing (persisting intent before mutation)
    journal
        .record_action_transition(action.action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(
            action.action_id,
            ActionState::Executing,
            Some("Persisting intent before filesystem mutation"),
        )
        .unwrap();

    // Sudden process termination simulated: no rename occurs!
    assert!(action.source_path.exists());
    assert!(!action.quarantine_path.exists());

    // Application restarts: run crash reconciliation
    let startup_inspection = inspect_startup_state(&journal).unwrap();
    assert!(!startup_inspection.is_clean);
    assert_eq!(startup_inspection.incomplete_actions.len(), 1);
    assert_eq!(startup_inspection.interrupted_sessions.len(), 1);

    let recovery_report = reconcile_startup_state(&mut journal).unwrap();
    assert_eq!(recovery_report.actions_checked, 1);
    assert_eq!(recovery_report.actions_resolved_failed, 1);
    assert_eq!(recovery_report.actions_resolved_completed, 0);
    assert_eq!(recovery_report.sessions_interrupted, 1);
    assert_eq!(
        recovery_report.details[0].resolution,
        ActionRecoveryResolution::UnmovedAtSource
    );

    // Assert file safety invariants:
    // 1. Source file is completely untouched with original content
    assert!(action.source_path.exists());
    assert_eq!(fs::read(&action.source_path).unwrap(), content);
    // 2. Quarantine path is clean
    assert!(!action.quarantine_path.exists());
    // 3. Keeper file is intact
    assert!(keeper_path.exists());
    assert_eq!(fs::read(&keeper_path).unwrap(), content);
    assert_eq!(dup_files.len(), 1);
}

#[test]
fn test_fault_injection_crash_after_filesystem_move_before_journal_commit() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Crash simulation: atomic move succeeded, process killed before SQLite commit";
    let (keeper_path, _dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let session_id = journal
        .create_session(&plan.root_dir, &plan.policy)
        .unwrap();
    let plan_id = journal.save_plan_for_session(session_id, &plan).unwrap();
    let plan_details = journal.fetch_plan(plan_id).unwrap();
    let action = &plan_details.actions[0];

    journal
        .record_action_transition(action.action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(
            action.action_id,
            ActionState::Executing,
            Some("Persisting intent before filesystem mutation"),
        )
        .unwrap();

    // Physical atomic move succeeds on disk
    fs::create_dir_all(action.quarantine_path.parent().unwrap()).unwrap();
    atomic_move_no_replace(&action.source_path, &action.quarantine_path).unwrap();

    // Process terminates before journal.record_action_transition(Completed) can run
    assert!(!action.source_path.exists());
    assert!(action.quarantine_path.exists());

    // Application restarts: run crash reconciliation
    let recovery_report = reconcile_startup_state(&mut journal).unwrap();
    assert_eq!(recovery_report.actions_resolved_completed, 1);
    assert_eq!(recovery_report.actions_resolved_failed, 0);
    assert_eq!(
        recovery_report.details[0].resolution,
        ActionRecoveryResolution::CompletedAtQuarantine
    );

    // Verify action state is now officially Completed in the SQLite journal
    let reloaded_actions = journal.list_actions_for_session(session_id).unwrap();
    assert_eq!(reloaded_actions[0].state, ActionState::Completed);

    // Now test conditional undo on this crash-recovered session!
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter.revert_session(&mut journal, session_id).unwrap();
    assert_eq!(undo_report.actions_restored, 1);
    assert_eq!(undo_report.actions_conflicted, 0);

    // Source duplicate restored cleanly to original location
    assert!(action.source_path.exists());
    assert_eq!(fs::read(&action.source_path).unwrap(), content);
    assert!(!action.quarantine_path.exists());
    assert!(keeper_path.exists());
}

#[test]
fn test_fault_injection_multi_action_batch_interruption() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Batch crash simulation with multiple actions in diverse failure states";
    let (keeper_path, dup_files, plan) = setup_test_files(&root, 4, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let session_id = journal
        .create_session(&plan.root_dir, &plan.policy)
        .unwrap();
    let plan_id = journal.save_plan_for_session(session_id, &plan).unwrap();
    let plan_details = journal.fetch_plan(plan_id).unwrap();
    assert_eq!(plan_details.actions.len(), 4);

    let actions = &plan_details.actions;

    // Action 0: Normal completed execution
    journal
        .record_action_transition(actions[0].action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(actions[0].action_id, ActionState::Executing, None)
        .unwrap();
    fs::create_dir_all(actions[0].quarantine_path.parent().unwrap()).unwrap();
    atomic_move_no_replace(&actions[0].source_path, &actions[0].quarantine_path).unwrap();
    journal
        .record_action_transition(actions[0].action_id, ActionState::Completed, None)
        .unwrap();

    // Action 1: Move completed on disk, journal crashed in 'Executing'
    journal
        .record_action_transition(actions[1].action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(actions[1].action_id, ActionState::Executing, None)
        .unwrap();
    fs::create_dir_all(actions[1].quarantine_path.parent().unwrap()).unwrap();
    atomic_move_no_replace(&actions[1].source_path, &actions[1].quarantine_path).unwrap();

    // Action 2: Intent recorded in 'Executing', process crashed before move
    journal
        .record_action_transition(actions[2].action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(actions[2].action_id, ActionState::Executing, None)
        .unwrap();

    // Action 3: Stuck in 'Validating', never executed
    journal
        .record_action_transition(actions[3].action_id, ActionState::Validating, None)
        .unwrap();

    // Run crash recovery reconciliation
    let recovery_report = reconcile_startup_state(&mut journal).unwrap();
    assert_eq!(recovery_report.actions_checked, 3); // actions 1, 2, 3 were incomplete
    assert_eq!(recovery_report.actions_resolved_completed, 1); // action 1
    assert_eq!(recovery_report.actions_resolved_failed, 2); // actions 2 and 3
    assert_eq!(recovery_report.sessions_interrupted, 1);

    // Run undo on the reconciled session:
    // Only Action 0 and Action 1 (which actually completed on disk) should be restored.
    // Actions 2 and 3 should be skipped safely without touching disk.
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter.revert_session(&mut journal, session_id).unwrap();
    assert_eq!(undo_report.actions_restored, 2);
    assert_eq!(undo_report.actions_skipped, 2);
    assert_eq!(undo_report.actions_conflicted, 0);

    // Verify all 4 files are safe and verified at their source paths!
    for dup in &dup_files {
        assert!(
            dup.exists(),
            "Duplicate file '{}' must exist",
            dup.display()
        );
        assert_eq!(fs::read(dup).unwrap(), content);
    }
    assert!(keeper_path.exists());
    assert_eq!(fs::read(&keeper_path).unwrap(), content);
}

// =========================================================================
// 2. Permission Drops & Filesystem Access Failures
// =========================================================================

#[test]
#[cfg(unix)]
fn test_fault_injection_permission_drop_source_unreadable() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Permission drop fault injection: source file made unreadable (0000)";
    let (keeper_path, dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    // Drop read permissions on the duplicate source file (0000)
    let _guard = PermissionGuard::new(&dup_files[0], 0o000, 0o644);

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    // Precondition verification must fail closed immediately
    assert_eq!(report.completed_actions, 0);
    assert_eq!(report.failed_actions, 1);
    assert_eq!(report.bytes_reclaimed, 0);

    // Keeper must be preserved untouched
    assert!(keeper_path.exists());
    // Quarantine destination must remain empty
    assert!(!plan.actions[0].quarantine_path.exists());
}

#[test]
#[cfg(unix)]
fn test_fault_injection_permission_drop_keeper_unreadable() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Permission drop fault injection: keeper file made unreadable (0000)";
    let (keeper_path, dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    // Drop read permissions on keeper (0000)
    let _guard = PermissionGuard::new(&keeper_path, 0o000, 0o644);

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    // Refuses quarantine because preserved keeper cannot be cryptographically verified
    assert_eq!(report.completed_actions, 0);
    assert_eq!(report.failed_actions, 1);
    assert_eq!(report.bytes_reclaimed, 0);

    // Source duplicate file is untouched
    assert!(dup_files[0].exists());
    assert!(!plan.actions[0].quarantine_path.exists());
}

#[test]
#[cfg(unix)]
fn test_fault_injection_permission_drop_quarantine_directory_unwritable() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Permission drop fault injection: quarantine directory made read-only (0555)";
    let (keeper_path, dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    // Pre-create the quarantine destination directory and revoke write permission (0555)
    let quarantine_parent = plan.actions[0].quarantine_path.parent().unwrap();
    fs::create_dir_all(quarantine_parent).unwrap();
    let _guard = PermissionGuard::new(quarantine_parent, 0o555, 0o755);

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    // Fails closed on atomic move / write denial
    assert_eq!(report.completed_actions, 0);
    assert_eq!(report.failed_actions, 1);

    // Source duplicate file remains safely intact at original location
    assert!(dup_files[0].exists());
    assert_eq!(fs::read(&dup_files[0]).unwrap(), content);
    assert!(keeper_path.exists());
}

#[test]
#[cfg(unix)]
fn test_fault_injection_permission_drop_undo_destination_unwritable() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Permission drop fault injection: source directory made unwritable before undo";
    let (keeper_path, _dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let source_path = plan.actions[0].source_path.clone();
    let quarantine_path = plan.actions[0].quarantine_path.clone();

    // 1. Quarantine duplicate normally
    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    assert_eq!(exec_report.completed_actions, 1);
    assert!(!source_path.exists());
    assert!(quarantine_path.exists());

    // 2. Make the source parent directory read-only (0555) before undo
    let source_parent = source_path.parent().unwrap();
    let _guard = PermissionGuard::new(source_parent, 0o555, 0o755);

    // 3. Attempt undo
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    // Fails closed: undo cannot restore into unwritable directory
    assert_eq!(undo_report.actions_restored, 0);
    assert_eq!(undo_report.actions_conflicted, 1);

    // Quarantined file is NOT lost or deleted; remains safely in quarantine!
    assert!(quarantine_path.exists());
    assert_eq!(fs::read(&quarantine_path).unwrap(), content);
    assert!(keeper_path.exists());
}

// =========================================================================
// 3. Missing Keeper Race Condition Tests
// =========================================================================

#[test]
fn test_fault_injection_missing_keeper_during_execution() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Missing keeper fault injection: keeper deleted right before execution";
    let (keeper_path, dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    // Delete keeper file to simulate concurrent deletion / race condition
    fs::remove_file(&keeper_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    // Precondition revalidation fails closed with KeeperMissing
    assert_eq!(report.completed_actions, 0);
    assert_eq!(report.failed_actions, 1);
    assert!(report.results[0]
        .status
        .to_string()
        .to_lowercase()
        .contains("keeper"));

    // Source duplicate must NOT be moved to quarantine
    assert!(dup_files[0].exists());
    assert_eq!(fs::read(&dup_files[0]).unwrap(), content);
    assert!(!plan.actions[0].quarantine_path.exists());
}

#[test]
fn test_fault_injection_missing_keeper_during_undo() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Missing keeper fault injection: keeper deleted while file is in quarantine";
    let (keeper_path, _dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let source_path = plan.actions[0].source_path.clone();
    let quarantine_path = plan.actions[0].quarantine_path.clone();

    // 1. Execute quarantine normally
    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    assert_eq!(exec_report.completed_actions, 1);
    assert!(keeper_path.exists());
    assert!(quarantine_path.exists());

    // 2. Keeper is deleted while file resides in quarantine
    fs::remove_file(&keeper_path).unwrap();

    // 3. Attempt undo
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    // Refuses to restore without verified keeper: logs conflict
    assert_eq!(undo_report.actions_restored, 0);
    assert_eq!(undo_report.actions_conflicted, 1);
    assert!(matches!(
        undo_report.conflicts[0].reason,
        UndoConflictReason::KeeperMissing { .. }
    ));

    // Quarantined file remains intact in quarantine; destination not modified
    assert!(quarantine_path.exists());
    assert!(!source_path.exists());
}

// =========================================================================
// 4. Destination Collision & Non-Overwrite Tests
// =========================================================================

#[test]
fn test_fault_injection_destination_collision_during_execution() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content = b"Destination collision fault injection: target path pre-occupied in quarantine";
    let foreign_content = b"Pre-existing rogue file in quarantine that must NEVER be overwritten";
    let (keeper_path, dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    // Plant a foreign file at the exact quarantine target path
    let quarantine_path = &plan.actions[0].quarantine_path;
    fs::create_dir_all(quarantine_path.parent().unwrap()).unwrap();
    fs::write(quarantine_path, foreign_content).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    // Non-replace invariant enforced: execution fails closed
    assert_eq!(report.completed_actions, 0);
    assert_eq!(report.failed_actions, 1);

    // 1. Foreign file in quarantine is completely intact
    assert_eq!(fs::read(quarantine_path).unwrap(), foreign_content);
    // 2. Source duplicate file was NOT deleted or overwritten
    assert!(dup_files[0].exists());
    assert_eq!(fs::read(&dup_files[0]).unwrap(), content);
    // 3. Keeper intact
    assert!(keeper_path.exists());
}

#[test]
fn test_fault_injection_destination_collision_during_undo() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content =
        b"Destination collision fault injection: source path occupied by new user file before undo";
    let new_user_content = b"Newly created user work that must NEVER be overwritten by an undo";
    let (keeper_path, _dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let source_path = plan.actions[0].source_path.clone();
    let quarantine_path = plan.actions[0].quarantine_path.clone();

    // 1. Execute quarantine normally
    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    assert_eq!(exec_report.completed_actions, 1);
    assert!(!source_path.exists());

    // 2. User creates a new file at the original duplicate path
    fs::write(&source_path, new_user_content).unwrap();

    // 3. Attempt undo
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    // Invariant: Never overwrite an existing destination merely to complete an undo
    assert_eq!(undo_report.actions_restored, 0);
    assert_eq!(undo_report.actions_conflicted, 1);
    assert!(matches!(
        undo_report.conflicts[0].reason,
        UndoConflictReason::DestinationOccupied { .. }
    ));

    // 1. User file at original path is NOT overwritten
    assert_eq!(fs::read(&source_path).unwrap(), new_user_content);
    // 2. Quarantined file remains safe in quarantine
    assert!(quarantine_path.exists());
    assert_eq!(fs::read(&quarantine_path).unwrap(), content);
    assert!(keeper_path.exists());
}

// =========================================================================
// 5. Corrupted Files, Tampering & Path Failure Tests
// =========================================================================

#[test]
fn test_fault_injection_corrupted_destination_file_recovery() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content =
        b"Crash corruption: destination file truncated/corrupted during sudden power kill";
    let (keeper_path, _dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let session_id = journal
        .create_session(&plan.root_dir, &plan.policy)
        .unwrap();
    let plan_id = journal.save_plan_for_session(session_id, &plan).unwrap();
    let plan_details = journal.fetch_plan(plan_id).unwrap();
    let action = &plan_details.actions[0];

    journal
        .record_action_transition(action.action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(action.action_id, ActionState::Executing, None)
        .unwrap();

    // Simulate power loss that leaves corrupted / 0-byte file at destination and removes source
    fs::create_dir_all(action.quarantine_path.parent().unwrap()).unwrap();
    let mut f = File::create(&action.quarantine_path).unwrap();
    f.write_all(b"partial/corrupted bytes").unwrap();
    fs::remove_file(&action.source_path).unwrap();

    // Run crash recovery reconciliation
    let recovery_report = reconcile_startup_state(&mut journal).unwrap();
    assert_eq!(recovery_report.actions_resolved_failed, 1);
    assert_eq!(recovery_report.actions_resolved_completed, 0);
    assert!(matches!(
        recovery_report.details[0].resolution,
        ActionRecoveryResolution::CorruptOrMissing { .. }
    ));

    // Must not falsely mark action Completed
    let reloaded = journal.list_actions_for_session(session_id).unwrap();
    assert_eq!(reloaded[0].state, ActionState::Failed);
    assert!(keeper_path.exists());
}

#[test]
fn test_fault_injection_tampered_quarantined_file_refuses_undo() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content =
        b"Tamper fault injection: payload modified by malware/external process in quarantine";
    let (keeper_path, _dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let source_path = plan.actions[0].source_path.clone();
    let quarantine_path = plan.actions[0].quarantine_path.clone();

    // 1. Execute quarantine normally
    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    assert_eq!(exec_report.completed_actions, 1);

    // 2. External actor tampers with bytes inside the quarantine directory (same length, flipped byte)
    let mut tampered_content = content.to_vec();
    tampered_content[0] ^= 0xFF; // bit flip ensures size matches but BLAKE3 hash fails
    fs::write(&quarantine_path, tampered_content).unwrap();

    // 3. Attempt undo
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    // Cryptographic BLAKE3 verification detects tampering and refuses restoration
    assert_eq!(undo_report.actions_restored, 0);
    assert_eq!(undo_report.actions_conflicted, 1);
    assert!(matches!(
        undo_report.conflicts[0].reason,
        UndoConflictReason::QuarantinedFileHashMismatch { .. }
    ));

    // Tampered file is NOT restored to user workspace
    assert!(!source_path.exists());
    assert!(keeper_path.exists());
}

#[test]
fn test_fault_injection_uncreatable_quarantine_path_fails_closed() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content =
        b"Path blockage fault injection: quarantine parent path is an occupied regular file";
    let (keeper_path, dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    // Block directory creation by creating a regular file named '.reclaim'
    let reclaim_dir_target = root.join(".reclaim");
    fs::write(&reclaim_dir_target, b"blocking regular file").unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    // Directory creation fails -> executor fails closed cleanly without panicking
    assert_eq!(report.completed_actions, 0);
    assert_eq!(report.failed_actions, 1);

    // Source duplicate is completely intact
    assert!(dup_files[0].exists());
    assert_eq!(fs::read(&dup_files[0]).unwrap(), content);
    assert!(keeper_path.exists());
}

#[test]
fn test_fault_injection_double_undo_idempotency() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let content =
        b"Idempotency fault injection: repeated undo calls must never double-move or corrupt state";
    let (_keeper, dup_files, plan) = setup_test_files(&root, 1, content);

    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    assert_eq!(exec_report.completed_actions, 1);

    let reverter = SafeReverter::new(UndoOptions::default());

    // First undo: succeeds
    let first_undo = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();
    assert_eq!(first_undo.actions_restored, 1);
    assert_eq!(first_undo.actions_skipped, 0);
    assert!(dup_files[0].exists());

    // Second undo on same session: skipped gracefully due to 'Reverted' state guard
    let second_undo = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();
    assert_eq!(second_undo.actions_restored, 0);
    assert_eq!(second_undo.actions_skipped, 1);
    assert!(
        second_undo.results[0].status
            == UndoStatus::Skipped {
                reason: "Action already reverted; skipping double-undo".to_string()
            }
    );
}
