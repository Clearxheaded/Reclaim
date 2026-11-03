use reclaim_core::duplicates::{find_duplicates, DuplicateOptions};
use reclaim_core::planner::{generate_plan, CleanupPlan, KeeperPolicy, PlanOptions};
use reclaim_core::scanner::{scan_directory, ScanOptions};
use reclaim_storage::executor::{ExecutionOptions, ExecutionProgressEvent, SafeExecutor};
use reclaim_storage::journal::ActionJournal;
use reclaim_storage::types::ActionState;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Helper to construct a synthetic test environment with 2 duplicate groups.
fn setup_duplicate_fixture(temp_dir: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let files_dir = temp_dir.join("files");
    fs::create_dir_all(&files_dir).unwrap();

    let doc_a = files_dir.join("doc_a.txt");
    let doc_b = files_dir.join("doc_b.txt");
    let photo_a = files_dir.join("photo_a.bin");
    let photo_b = files_dir.join("photo_b.bin");

    fs::write(&doc_a, b"identical document text content").unwrap();
    fs::write(&doc_b, b"identical document text content").unwrap();

    fs::write(&photo_a, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRbinarydata1").unwrap();
    fs::write(&photo_b, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRbinarydata1").unwrap();

    (doc_a, doc_b, photo_a, photo_b)
}

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

#[test]
fn test_executor_happy_path() {
    let temp = TempDir::new().unwrap();
    let (doc_a, doc_b, photo_a, photo_b) = setup_duplicate_fixture(temp.path());
    let quarantine_dir = temp.path().join("quarantine");

    let plan = create_test_plan(temp.path(), &quarantine_dir);
    assert_eq!(plan.actions.len(), 2);
    assert_eq!(plan.keepers.len(), 2);

    let db_path = temp.path().join("reclaim.journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    // Verify execution metrics
    assert_eq!(report.total_actions, 2);
    assert_eq!(report.completed_actions, 2);
    assert_eq!(report.failed_actions, 0);
    assert_eq!(report.skipped_actions, 0);
    assert_eq!(report.bytes_reclaimed, plan.summary.total_reclaimable_bytes);

    // Keepers must remain intact at their original paths
    assert!(doc_a.exists(), "Keeper doc_a must remain intact");
    assert!(photo_a.exists(), "Keeper photo_a must remain intact");

    // Duplicates must have been moved to quarantine
    assert!(!doc_b.exists(), "Duplicate doc_b should be quarantined");
    assert!(!photo_b.exists(), "Duplicate photo_b should be quarantined");

    let target_doc_b = plan
        .actions
        .iter()
        .find(|a| a.source_path == doc_b)
        .unwrap();
    let target_photo_b = plan
        .actions
        .iter()
        .find(|a| a.source_path == photo_b)
        .unwrap();
    assert!(
        target_doc_b.quarantine_path.exists(),
        "Quarantined doc_b must exist at staged path"
    );
    assert!(
        target_photo_b.quarantine_path.exists(),
        "Quarantined photo_b must exist at staged path"
    );
    assert_eq!(
        fs::read(&target_doc_b.quarantine_path).unwrap(),
        b"identical document text content"
    );

    // Verify journal persistence and audit events
    let fetched_plan = journal.fetch_plan(report.plan_id).unwrap();
    for action in &fetched_plan.actions {
        assert_eq!(
            action.state,
            ActionState::Completed,
            "All actions must be in Completed state"
        );

        let audit = journal.fetch_action_audit_trail(action.action_id).unwrap();
        // Planned -> Validating -> Executing -> Completed
        assert_eq!(audit.len(), 4);
        assert_eq!(audit[0].to_state, ActionState::Planned);
        assert_eq!(audit[1].to_state, ActionState::Validating);
        assert_eq!(audit[2].to_state, ActionState::Executing);
        assert_eq!(audit[3].to_state, ActionState::Completed);
    }
}

#[test]
fn test_executor_dry_run_zero_filesystem_mutations() {
    let temp = TempDir::new().unwrap();
    let (doc_a, doc_b, photo_a, photo_b) = setup_duplicate_fixture(temp.path());
    let quarantine_dir = temp.path().join("quarantine");

    let plan = create_test_plan(temp.path(), &quarantine_dir);
    assert_eq!(plan.actions.len(), 2);

    let db_path = temp.path().join("reclaim.journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let options = ExecutionOptions {
        dry_run: true,
        halt_on_error: false,
    };
    let mut executor = SafeExecutor::new(options);
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    assert_eq!(report.total_actions, 2);
    assert_eq!(report.completed_actions, 2);
    assert_eq!(report.failed_actions, 0);
    assert!(report.dry_run);

    // Non-negotiable invariant: Zero filesystem modifications in dry-run mode!
    assert!(doc_a.exists(), "doc_a must exist");
    assert!(doc_b.exists(), "doc_b must NOT be moved in dry-run");
    assert!(photo_a.exists(), "photo_a must exist");
    assert!(photo_b.exists(), "photo_b must NOT be moved in dry-run");

    // Quarantine folder must not even be created in dry-run
    assert!(
        !quarantine_dir.exists(),
        "Quarantine directory must NOT be created on disk during dry-run"
    );

    // Journal should reflect simulation
    let fetched = journal.fetch_plan(report.plan_id).unwrap();
    for action in &fetched.actions {
        assert_eq!(action.state, ActionState::Completed);
        let audit = journal.fetch_action_audit_trail(action.action_id).unwrap();
        let last_event = audit.last().unwrap();
        assert!(last_event.details.as_ref().unwrap().contains("Dry run"));
    }
}

#[test]
fn test_executor_destination_collision_fails_closed() {
    let temp = TempDir::new().unwrap();
    let (_doc_a, doc_b, _photo_a, _photo_b) = setup_duplicate_fixture(temp.path());
    let quarantine_dir = temp.path().join("quarantine");

    let plan = create_test_plan(temp.path(), &quarantine_dir);

    // Seed a file at the exact quarantine destination path of doc_b
    let target_action = plan
        .actions
        .iter()
        .find(|a| a.source_path == doc_b)
        .unwrap();
    fs::create_dir_all(target_action.quarantine_path.parent().unwrap()).unwrap();
    fs::write(
        &target_action.quarantine_path,
        b"unrelated user file sitting at quarantine path",
    )
    .unwrap();

    let db_path = temp.path().join("reclaim.journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    assert_eq!(report.total_actions, 2);
    assert_eq!(report.completed_actions, 1);
    assert_eq!(report.failed_actions, 1);

    // Verify colliding destination was NOT overwritten
    assert_eq!(
        fs::read(&target_action.quarantine_path).unwrap(),
        b"unrelated user file sitting at quarantine path",
        "Pre-existing file at quarantine path must NEVER be overwritten!"
    );

    // Verify source duplicate was NOT deleted
    assert!(
        doc_b.exists(),
        "Source duplicate must remain intact when destination collides"
    );

    // Verify journal recorded failure
    let fetched = journal.fetch_plan(report.plan_id).unwrap();
    let failed_action = fetched
        .actions
        .iter()
        .find(|a| a.source_path == doc_b)
        .unwrap();
    assert_eq!(failed_action.state, ActionState::Failed);

    let audit = journal
        .fetch_action_audit_trail(failed_action.action_id)
        .unwrap();
    let last_event = audit.last().unwrap();
    assert_eq!(last_event.to_state, ActionState::Failed);
    assert!(last_event
        .details
        .as_ref()
        .unwrap()
        .contains("already exists"));
}

#[test]
fn test_executor_stale_content_modification_detected() {
    let temp = TempDir::new().unwrap();
    let (_doc_a, doc_b, _photo_a, _photo_b) = setup_duplicate_fixture(temp.path());
    let quarantine_dir = temp.path().join("quarantine");

    let plan = create_test_plan(temp.path(), &quarantine_dir);

    // Tamper with doc_b content between planning and execution (TOCTOU simulation)
    fs::write(&doc_b, b"tampered text content that violates hash").unwrap();

    let db_path = temp.path().join("reclaim.journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    assert_eq!(report.completed_actions, 1);
    assert_eq!(report.failed_actions, 1);

    // Modified file must not have been moved
    assert!(doc_b.exists(), "Tampered file must remain untouched");

    let fetched = journal.fetch_plan(report.plan_id).unwrap();
    let tampered_action = fetched
        .actions
        .iter()
        .find(|a| a.source_path == doc_b)
        .unwrap();
    assert_eq!(tampered_action.state, ActionState::Failed);

    let audit = journal
        .fetch_action_audit_trail(tampered_action.action_id)
        .unwrap();
    let last = audit.last().unwrap();
    assert!(
        last.details.as_ref().unwrap().contains("mismatch"),
        "Audit event should record hash/size mismatch"
    );
}

#[test]
fn test_executor_missing_keeper_refuses_quarantine() {
    let temp = TempDir::new().unwrap();
    let (doc_a, doc_b, _photo_a, _photo_b) = setup_duplicate_fixture(temp.path());
    let quarantine_dir = temp.path().join("quarantine");

    let plan = create_test_plan(temp.path(), &quarantine_dir);

    // Delete the preserved keeper file doc_a
    fs::remove_file(&doc_a).unwrap();

    let db_path = temp.path().join("reclaim.journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    assert_eq!(report.failed_actions, 1);
    assert_eq!(report.completed_actions, 1);

    // The duplicate whose keeper vanished must NOT be moved
    assert!(
        doc_b.exists(),
        "Duplicate must not be quarantined if keeper is missing"
    );

    let fetched = journal.fetch_plan(report.plan_id).unwrap();
    let failed_action = fetched
        .actions
        .iter()
        .find(|a| a.source_path == doc_b)
        .unwrap();
    assert_eq!(failed_action.state, ActionState::Failed);

    let audit = journal
        .fetch_action_audit_trail(failed_action.action_id)
        .unwrap();
    let last = audit.last().unwrap();
    assert!(last
        .details
        .as_ref()
        .unwrap()
        .contains("keeper file missing"));
}

#[cfg(unix)]
#[test]
fn test_executor_symlink_defense() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let (doc_a, doc_b, _photo_a, _photo_b) = setup_duplicate_fixture(temp.path());
    let quarantine_dir = temp.path().join("quarantine");

    let plan = create_test_plan(temp.path(), &quarantine_dir);

    // Replace doc_b duplicate with a symlink pointing to doc_a
    fs::remove_file(&doc_b).unwrap();
    symlink(&doc_a, &doc_b).unwrap();

    let db_path = temp.path().join("reclaim.journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor.execute_plan(&mut journal, &plan).unwrap();

    assert_eq!(report.failed_actions, 1);

    // Symlink must not have been traversed or moved
    assert!(
        fs::symlink_metadata(&doc_b)
            .unwrap()
            .file_type()
            .is_symlink(),
        "Symlink duplicate must remain untouched"
    );

    let fetched = journal.fetch_plan(report.plan_id).unwrap();
    let failed = fetched
        .actions
        .iter()
        .find(|a| a.source_path == doc_b)
        .unwrap();
    assert_eq!(failed.state, ActionState::Failed);
    let audit = journal.fetch_action_audit_trail(failed.action_id).unwrap();
    assert!(audit
        .last()
        .unwrap()
        .details
        .as_ref()
        .unwrap()
        .contains("symlink"));
}

#[test]
fn test_executor_progress_callback() {
    let temp = TempDir::new().unwrap();
    let (_doc_a, _doc_b, _photo_a, _photo_b) = setup_duplicate_fixture(temp.path());
    let quarantine_dir = temp.path().join("quarantine");

    let plan = create_test_plan(temp.path(), &quarantine_dir);

    let db_path = temp.path().join("reclaim.journal.db");
    let mut journal = ActionJournal::open(&db_path).unwrap();

    let mut events_seen = Vec::new();
    let mut executor = SafeExecutor::new(ExecutionOptions::default());
    let report = executor
        .execute_plan_with_callback(&mut journal, &plan, |event| match event {
            ExecutionProgressEvent::Validating { index, total, .. } => {
                events_seen.push(format!("validating {}/{}", index, total));
            }
            ExecutionProgressEvent::Executing { dry_run, .. } => {
                events_seen.push(format!("executing (dry_run: {})", dry_run));
            }
            ExecutionProgressEvent::Completed { bytes_freed, .. } => {
                events_seen.push(format!("completed ({} bytes)", bytes_freed));
            }
            _ => {}
        })
        .unwrap();

    assert_eq!(report.completed_actions, 2);

    // Verify progress events were fired in sequence
    assert_eq!(events_seen.len(), 6); // 2 * (validating + executing + completed)
    assert_eq!(events_seen[0], "validating 1/2");
    assert_eq!(events_seen[1], "executing (dry_run: false)");
    assert!(events_seen[2].starts_with("completed"));
}
