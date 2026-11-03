use reclaim_core::duplicates::{find_duplicates, DuplicateOptions};
use reclaim_core::planner::{generate_plan, CleanupPlan, KeeperPolicy, PlanOptions};
use reclaim_core::scanner::{scan_directory, ScanOptions};
use reclaim_storage::undo::{
    SafeReverter, UndoConflictReason, UndoOptions, UndoProgressEvent, UndoStatus,
};
use reclaim_storage::{ActionJournal, ActionState, ExecutionOptions, SafeExecutor};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Helper to generate a `CleanupPlan` from a test directory.
fn create_test_plan(root_dir: &Path, quarantine_dir: &Path) -> CleanupPlan {
    let scan_opts = ScanOptions::default();
    let scan_out = scan_directory(root_dir, scan_opts).unwrap();

    let dup_opts = DuplicateOptions::default();
    let dup_out = find_duplicates(&scan_out.entries, &dup_opts);

    let plan_opts = PlanOptions {
        root_dir: root_dir.to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: Some(quarantine_dir.to_path_buf()),
    };

    generate_plan(&dup_out.groups, &plan_opts).unwrap()
}

/// Helper building a real directory structure, plan, and populated files.
fn setup_test_environment(content: &[u8]) -> (TempDir, PathBuf, CleanupPlan, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let canonical_dir = root.join("canonical");
    let copies_dir = root.join("copies");
    fs::create_dir_all(&canonical_dir).unwrap();
    fs::create_dir_all(&copies_dir).unwrap();

    let keeper_file = canonical_dir.join("keeper.txt");
    let duplicate_file = copies_dir.join("duplicate.txt");

    fs::write(&keeper_file, content).unwrap();
    fs::write(&duplicate_file, content).unwrap();

    let quarantine_dir = root.join(".reclaim").join("quarantine");
    let plan = create_test_plan(&root, &quarantine_dir);

    (temp, root, plan, keeper_file, duplicate_file)
}

#[test]
fn test_undo_happy_path_full_lifecycle() {
    let content = b"Exact duplicate payload for happy path full undo test 987654";
    let (_temp, _root, plan, keeper_path, duplicate_path) = setup_test_environment(content);

    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    // 1. Execute quarantine
    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    assert_eq!(exec_report.completed_actions, 1);
    assert_eq!(exec_report.failed_actions, 0);

    // Verify file was quarantined
    assert!(
        !duplicate_path.exists(),
        "Source file should have been moved to quarantine"
    );
    let actions = journal
        .list_actions_for_session(exec_report.session_id)
        .unwrap();
    assert_eq!(actions[0].state, ActionState::Completed);
    assert!(actions[0].quarantine_path.exists());

    // 2. Perform Undo
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    assert_eq!(undo_report.actions_restored, 1);
    assert_eq!(undo_report.actions_conflicted, 0);
    assert_eq!(undo_report.actions_skipped, 0);
    assert_eq!(undo_report.bytes_restored, content.len() as u64);

    // Verify file returned to original location with identical content
    assert!(
        duplicate_path.exists(),
        "File should be restored to original path"
    );
    let restored_content = fs::read(&duplicate_path).unwrap();
    assert_eq!(restored_content, content);

    // Verify quarantine file is gone
    assert!(
        !actions[0].quarantine_path.exists(),
        "Quarantine file should have been restored"
    );

    // Verify keeper is still intact
    assert!(keeper_path.exists());

    // Verify state machine transition in SQLite journal
    let updated_action = journal.get_action(actions[0].action_id).unwrap();
    assert_eq!(updated_action.state, ActionState::Reverted);

    // Verify audit trail recorded Completed -> Reverted
    let audit_events = journal
        .fetch_action_audit_trail(actions[0].action_id)
        .unwrap();
    let last_event = audit_events.last().unwrap();
    assert_eq!(last_event.from_state, Some(ActionState::Completed));
    assert_eq!(last_event.to_state, ActionState::Reverted);
}

#[test]
fn test_undo_conflict_destination_occupied() {
    let content = b"Duplicate content for destination collision defense test";
    let (_temp, _root, plan, _keeper_path, duplicate_path) = setup_test_environment(content);

    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    // Execute quarantine
    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    assert_eq!(exec_report.completed_actions, 1);
    assert!(!duplicate_path.exists());

    let actions = journal
        .list_actions_for_session(exec_report.session_id)
        .unwrap();
    assert!(actions[0].quarantine_path.exists());

    // ATTACK / CONFLICT SCENARIO:
    // A new file was placed at the original duplicate path before undo!
    let foreign_content = b"FOREIGN OCCUPYING FILE CONTENT - MUST NEVER BE OVERWRITTEN!";
    fs::write(&duplicate_path, foreign_content).unwrap();

    // Perform Undo
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    // Verify safety invariants:
    // 1. Undo refused to overwrite
    assert_eq!(undo_report.actions_restored, 0);
    assert_eq!(undo_report.actions_conflicted, 1);
    assert!(matches!(
        undo_report.conflicts[0].reason,
        UndoConflictReason::DestinationOccupied { .. }
    ));

    // 2. The foreign file at destination is completely untouched
    let dest_content = fs::read(&duplicate_path).unwrap();
    assert_eq!(dest_content, foreign_content);

    // 3. The quarantined file remains safe in quarantine
    assert!(actions[0].quarantine_path.exists());
    let q_content = fs::read(&actions[0].quarantine_path).unwrap();
    assert_eq!(q_content, content);

    // 4. Action state remains Completed (NOT Reverted)
    let updated_action = journal.get_action(actions[0].action_id).unwrap();
    assert_eq!(updated_action.state, ActionState::Completed);

    // 5. Audit trail logged the conflict event
    let events = journal
        .fetch_action_audit_trail(actions[0].action_id)
        .unwrap();
    let conflict_logged = events.iter().any(|e| {
        e.details
            .as_ref()
            .map(|d| d.contains("Destination path already exists"))
            .unwrap_or(false)
    });
    assert!(
        conflict_logged,
        "Audit log must record destination collision conflict"
    );
}

#[test]
fn test_undo_conflict_tampered_quarantine_file() {
    let content = b"Original payload before tampering";
    let (_temp, _root, plan, _keeper_path, duplicate_path) = setup_test_environment(content);

    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    let actions = journal
        .list_actions_for_session(exec_report.session_id)
        .unwrap();

    // Tamper with the quarantined file content
    fs::write(
        &actions[0].quarantine_path,
        b"Tampered payload with different hash",
    )
    .unwrap();

    // Attempt Undo
    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    assert_eq!(undo_report.actions_restored, 0);
    assert_eq!(undo_report.actions_conflicted, 1);
    assert!(matches!(
        undo_report.conflicts[0].reason,
        UndoConflictReason::QuarantinedFileHashMismatch { .. }
            | UndoConflictReason::QuarantinedFileSizeMismatch { .. }
    ));

    assert!(
        !duplicate_path.exists(),
        "Tampered file must not be restored to source"
    );
    let updated_action = journal.get_action(actions[0].action_id).unwrap();
    assert_eq!(updated_action.state, ActionState::Completed);
}

#[test]
fn test_undo_conflict_missing_quarantine_file() {
    let content = b"Payload whose quarantine copy gets deleted";
    let (_temp, _root, plan, _keeper_path, _duplicate_path) = setup_test_environment(content);

    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    let actions = journal
        .list_actions_for_session(exec_report.session_id)
        .unwrap();

    // Manually delete the quarantined file
    fs::remove_file(&actions[0].quarantine_path).unwrap();

    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    assert_eq!(undo_report.actions_restored, 0);
    assert_eq!(undo_report.actions_conflicted, 1);
    assert!(matches!(
        undo_report.conflicts[0].reason,
        UndoConflictReason::QuarantinedFileMissing { .. }
    ));
}

#[test]
fn test_undo_conflict_missing_keeper() {
    let content = b"Payload whose keeper is deleted";
    let (_temp, _root, plan, keeper_path, duplicate_path) = setup_test_environment(content);

    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    let actions = journal
        .list_actions_for_session(exec_report.session_id)
        .unwrap();

    // Delete the keeper file!
    fs::remove_file(&keeper_path).unwrap();

    let reverter = SafeReverter::new(UndoOptions::default());
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    assert_eq!(undo_report.actions_restored, 0);
    assert_eq!(undo_report.actions_conflicted, 1);
    assert!(matches!(
        undo_report.conflicts[0].reason,
        UndoConflictReason::KeeperMissing { .. }
    ));

    assert!(!duplicate_path.exists());
    assert!(actions[0].quarantine_path.exists());
}

#[test]
fn test_undo_dry_run_zero_filesystem_mutations() {
    let content = b"Payload for dry-run undo simulation testing";
    let (_temp, _root, plan, _keeper_path, duplicate_path) = setup_test_environment(content);

    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
    let actions = journal
        .list_actions_for_session(exec_report.session_id)
        .unwrap();

    // Run Undo with dry_run = true
    let reverter = SafeReverter::new(UndoOptions {
        dry_run: true,
        halt_on_error: false,
    });
    let undo_report = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();

    assert!(undo_report.dry_run);
    assert_eq!(undo_report.actions_restored, 1);
    assert_eq!(undo_report.actions_conflicted, 0);
    assert_eq!(undo_report.results[0].status, UndoStatus::DryRunValidated);

    // Verify ZERO filesystem mutations were performed
    assert!(
        !duplicate_path.exists(),
        "Dry run must NOT restore file to disk"
    );
    assert!(
        actions[0].quarantine_path.exists(),
        "Dry run must leave quarantine file intact"
    );

    // Verify journal state was NOT changed
    let action_after = journal.get_action(actions[0].action_id).unwrap();
    assert_eq!(action_after.state, ActionState::Completed);
}

#[test]
fn test_undo_idempotency_and_reversion_lock() {
    let content = b"Idempotency test payload";
    let (_temp, _root, plan, _keeper_path, duplicate_path) = setup_test_environment(content);

    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();

    let reverter = SafeReverter::new(UndoOptions::default());

    // First undo: succeeds
    let report1 = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();
    assert_eq!(report1.actions_restored, 1);
    assert!(duplicate_path.exists());

    // Second undo on same session: skips already reverted actions
    let report2 = reverter
        .revert_session(&mut journal, exec_report.session_id)
        .unwrap();
    assert_eq!(report2.actions_restored, 0);
    assert_eq!(report2.actions_skipped, 1);
    assert_eq!(report2.actions_conflicted, 0);
    assert!(matches!(
        report2.results[0].status,
        UndoStatus::Skipped { .. }
    ));
}

#[test]
fn test_undo_progress_callbacks() {
    let content = b"Callback test payload";
    let (_temp, _root, plan, _keeper_path, _duplicate_path) = setup_test_environment(content);

    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();

    let reverter = SafeReverter::new(UndoOptions::default());
    let mut events = Vec::new();

    reverter
        .revert_session_with_callback(&mut journal, exec_report.session_id, |event| match event {
            UndoProgressEvent::Validating { .. } => events.push("validating"),
            UndoProgressEvent::Restoring { .. } => events.push("restoring"),
            UndoProgressEvent::Restored { .. } => events.push("restored"),
            _ => {}
        })
        .unwrap();

    assert_eq!(events, vec!["validating", "restoring", "restored"]);
}
