use reclaim_core::duplicates::{find_duplicates, DuplicateOptions};
use reclaim_core::planner::{generate_plan, CleanupPlan, KeeperPolicy, PlanOptions};
use reclaim_core::scanner::{scan_directory, ScanOptions};
use reclaim_storage::recovery::{
    inspect_startup_state, reconcile_startup_state, ActionRecoveryResolution,
};
use reclaim_storage::{ActionJournal, ActionState, SessionStatus};
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

/// Helper building a real cleanup plan in a temp directory.
fn create_test_environment() -> (TempDir, PathBuf, CleanupPlan) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let canonical_dir = root.join("canonical");
    let copies_dir = root.join("copies");
    fs::create_dir_all(&canonical_dir).unwrap();
    fs::create_dir_all(&copies_dir).unwrap();

    let file_a = canonical_dir.join("original.txt");
    let file_b = copies_dir.join("dup.txt");

    let content = b"Exact duplicate payload for crash reconciliation testing 12345";
    fs::write(&file_a, content).unwrap();
    fs::write(&file_b, content).unwrap();

    let quarantine_dir = root.join(".reclaim").join("quarantine");
    let plan = create_test_plan(&root, &quarantine_dir);

    (temp, root, plan)
}

#[test]
fn test_recovery_clean_journal() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let startup = inspect_startup_state(&journal).unwrap();
    assert!(startup.is_clean);
    assert_eq!(startup.incomplete_actions.len(), 0);
    assert_eq!(startup.interrupted_sessions.len(), 0);

    let report = reconcile_startup_state(&mut journal).unwrap();
    assert!(report.is_clean);
    assert_eq!(report.actions_checked, 0);
    assert_eq!(report.actions_resolved_completed, 0);
    assert_eq!(report.actions_resolved_failed, 0);
    assert_eq!(report.sessions_interrupted, 0);
}

#[test]
fn test_recovery_interrupted_session_without_actions() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let session_id = journal
        .create_session(temp.path(), &KeeperPolicy::ShortestPath)
        .unwrap();

    let startup = inspect_startup_state(&journal).unwrap();
    assert!(!startup.is_clean);
    assert_eq!(startup.interrupted_sessions.len(), 1);

    let report = reconcile_startup_state(&mut journal).unwrap();
    assert!(!report.is_clean);
    assert_eq!(report.sessions_interrupted, 1);
    assert_eq!(report.actions_checked, 0);

    let session = journal.fetch_session_history(session_id).unwrap();
    assert_eq!(session.status, SessionStatus::Interrupted);

    // Second recovery should now be clean
    let post_report = reconcile_startup_state(&mut journal).unwrap();
    assert!(post_report.is_clean);
}

#[test]
fn test_recovery_move_completed_on_disk_before_crash() {
    let (_temp, _root, plan) = create_test_environment();
    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let plan_id = journal.save_plan(&plan).unwrap();
    let actions = journal.list_actions_for_plan(plan_id).unwrap();
    let action = &actions[0];

    // Simulate crash scenario 1:
    // The atomic move succeeded on disk (file moved from source to quarantine),
    // but the application crashed BEFORE recording ActionState::Completed in SQLite.
    // The action is left in Executing state.
    journal
        .record_action_transition(action.action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(
            action.action_id,
            ActionState::Executing,
            Some("Mutation started; simulating crash"),
        )
        .unwrap();

    // Physically move file to quarantine destination on disk
    fs::create_dir_all(action.quarantine_path.parent().unwrap()).unwrap();
    fs::rename(&action.source_path, &action.quarantine_path).unwrap();
    assert!(!action.source_path.exists());
    assert!(action.quarantine_path.exists());

    // Run active dual reconciliation
    let report = reconcile_startup_state(&mut journal).unwrap();
    assert_eq!(report.actions_checked, 1);
    assert_eq!(report.actions_resolved_completed, 1);
    assert_eq!(report.actions_resolved_failed, 0);
    assert_eq!(
        report.details[0].resolution,
        ActionRecoveryResolution::CompletedAtQuarantine
    );

    // Verify action state transitioned to Completed in the durable journal
    let updated_action = journal.get_action(action.action_id).unwrap();
    assert_eq!(updated_action.state, ActionState::Completed);

    // Verify audit trail logged the reconciliation explanation
    let events = journal.fetch_action_audit_trail(action.action_id).unwrap();
    let last_event = events.last().unwrap();
    assert_eq!(last_event.to_state, ActionState::Completed);
    assert!(last_event
        .details
        .as_ref()
        .unwrap()
        .contains("Crash recovery: verified atomic move completed on disk"));
}

#[test]
fn test_recovery_move_never_executed_file_still_at_source() {
    let (_temp, _root, plan) = create_test_environment();
    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let plan_id = journal.save_plan(&plan).unwrap();
    let actions = journal.list_actions_for_plan(plan_id).unwrap();
    let action = &actions[0];

    // Simulate crash scenario 2:
    // Process transitioned to Executing, but was killed BEFORE the rename syscall.
    // File is still at source; quarantine path does not exist.
    journal
        .record_action_transition(action.action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(
            action.action_id,
            ActionState::Executing,
            Some("Intent recorded; killed before rename syscall"),
        )
        .unwrap();

    assert!(action.source_path.exists());
    assert!(!action.quarantine_path.exists());

    // Run reconciliation
    let report = reconcile_startup_state(&mut journal).unwrap();
    assert_eq!(report.actions_checked, 1);
    assert_eq!(report.actions_resolved_completed, 0);
    assert_eq!(report.actions_resolved_failed, 1);
    assert_eq!(
        report.details[0].resolution,
        ActionRecoveryResolution::UnmovedAtSource
    );

    // Verify action transitioned to Failed
    let updated_action = journal.get_action(action.action_id).unwrap();
    assert_eq!(updated_action.state, ActionState::Failed);

    let events = journal.fetch_action_audit_trail(action.action_id).unwrap();
    let last_event = events.last().unwrap();
    assert_eq!(last_event.to_state, ActionState::Failed);
    assert!(last_event
        .details
        .as_ref()
        .unwrap()
        .contains("physical file remains at source path"));
}

#[test]
fn test_recovery_file_missing_at_both_paths() {
    let (_temp, _root, plan) = create_test_environment();
    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let plan_id = journal.save_plan(&plan).unwrap();
    let actions = journal.list_actions_for_plan(plan_id).unwrap();
    let action = &actions[0];

    // Seed in Executing
    journal
        .record_action_transition(action.action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(action.action_id, ActionState::Executing, None)
        .unwrap();

    // Delete source file; quarantine file does not exist either
    fs::remove_file(&action.source_path).unwrap();
    assert!(!action.source_path.exists());
    assert!(!action.quarantine_path.exists());

    let report = reconcile_startup_state(&mut journal).unwrap();
    assert_eq!(report.actions_resolved_failed, 1);
    assert!(matches!(
        report.details[0].resolution,
        ActionRecoveryResolution::CorruptOrMissing { .. }
    ));

    let updated_action = journal.get_action(action.action_id).unwrap();
    assert_eq!(updated_action.state, ActionState::Failed);
}

#[test]
fn test_recovery_file_corrupted_hash_mismatch() {
    let (_temp, _root, plan) = create_test_environment();
    let temp_db = TempDir::new().unwrap();
    let db_path = temp_db.path().join("journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let plan_id = journal.save_plan(&plan).unwrap();
    let actions = journal.list_actions_for_plan(plan_id).unwrap();
    let action = &actions[0];

    journal
        .record_action_transition(action.action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(action.action_id, ActionState::Executing, None)
        .unwrap();

    // Corrupt the file at source (modify content)
    fs::write(&action.source_path, b"Corrupted content with hash mismatch").unwrap();

    let report = reconcile_startup_state(&mut journal).unwrap();
    assert_eq!(report.actions_resolved_failed, 1);

    let updated_action = journal.get_action(action.action_id).unwrap();
    assert_eq!(updated_action.state, ActionState::Failed);
}
