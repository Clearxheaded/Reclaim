use reclaim_core::file_id::FileId;
use reclaim_core::planner::{
    ActionPreconditions, CleanupPlan, KeeperPolicy, PlanSummary, ProtectedKeeper, QuarantineAction,
};
use reclaim_storage::{
    inspect_startup_state, reconcile_interrupted_sessions, ActionJournal, ActionState,
    SessionStatus, StorageError,
};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tempfile::NamedTempFile;

/// Helper to generate a realistic mock cleanup plan.
fn create_mock_plan() -> CleanupPlan {
    let now = SystemTime::now();
    let root = PathBuf::from("/Users/alice/photos");
    let keeper_path = root.join("2026/vacation.jpg");
    let dup1_path = root.join("staging/vacation_copy.jpg");
    let dup2_path = root.join("backup/vacation_backup.jpg");

    let fid_keeper = FileId::Unix {
        dev: 16777220,
        ino: 1234567,
    };
    let fid_dup1 = FileId::Unix {
        dev: 16777220,
        ino: 7654321,
    };
    let fid_dup2 = FileId::Unix {
        dev: 16777220,
        ino: 8901234,
    };

    let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string();

    let keeper = ProtectedKeeper {
        path: keeper_path.clone(),
        size: 2048,
        hash: hash.clone(),
        file_id: Some(fid_keeper),
        modified: Some(now - Duration::from_secs(3600)),
        hard_link_aliases: vec![root.join("links/vacation_hardlink.jpg")],
        rationale: "Preserved shortest path under ShortestPath policy".to_string(),
    };

    let action1 = QuarantineAction {
        source_path: dup1_path.clone(),
        quarantine_path: root.join(".reclaim/quarantine/staging/vacation_copy.jpg"),
        keeper_path: keeper_path.clone(),
        preconditions: ActionPreconditions {
            expected_size: 2048,
            expected_hash: hash.clone(),
            source_modified: Some(now - Duration::from_secs(1800)),
            source_file_id: Some(fid_dup1),
            keeper_modified: Some(now - Duration::from_secs(3600)),
            keeper_file_id: Some(fid_keeper),
        },
        rationale: "Exact duplicate of keeper 2026/vacation.jpg".to_string(),
    };

    let action2 = QuarantineAction {
        source_path: dup2_path.clone(),
        quarantine_path: root.join(".reclaim/quarantine/backup/vacation_backup.jpg"),
        keeper_path: keeper_path.clone(),
        preconditions: ActionPreconditions {
            expected_size: 2048,
            expected_hash: hash,
            source_modified: Some(now - Duration::from_secs(900)),
            source_file_id: Some(fid_dup2),
            keeper_modified: Some(now - Duration::from_secs(3600)),
            keeper_file_id: Some(fid_keeper),
        },
        rationale: "Exact duplicate of keeper 2026/vacation.jpg".to_string(),
    };

    let summary = PlanSummary {
        total_duplicate_files: 3,
        duplicate_groups_count: 1,
        planned_actions: 2,
        protected_keepers: 1,
        total_reclaimable_bytes: 4096,
        files_left_untouched: 2,
    };

    CleanupPlan {
        created_at: now,
        root_dir: root,
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: PathBuf::from("/Users/alice/photos/.reclaim/quarantine"),
        actions: vec![action1, action2],
        keepers: vec![keeper],
        summary,
    }
}

#[test]
fn test_in_memory_journal_creation_and_pragmas() {
    let journal = ActionJournal::open_in_memory().expect("failed to open in-memory journal");
    let conn = journal.connection();

    // Verify foreign keys are enabled
    let fk: i64 = conn
        .query_row("PRAGMA foreign_keys;", [], |r| r.get(0))
        .expect("query foreign_keys pragma");
    assert_eq!(fk, 1, "Foreign keys must be enabled");

    // Verify schema version is recorded
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version;", [], |r| r.get(0))
        .expect("query schema version");
    assert_eq!(version, 1);
}

#[test]
fn test_disk_journal_wal_mode() {
    let temp_file = NamedTempFile::new().expect("create temp file");
    let path = temp_file.path().to_path_buf();

    let journal = ActionJournal::open(&path).expect("failed to open disk journal");
    let conn = journal.connection();

    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
        .expect("query journal_mode");
    assert_eq!(
        journal_mode.to_lowercase(),
        "wal",
        "Disk journal must use WAL mode"
    );
}

#[test]
fn test_session_lifecycle_and_history() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let root = PathBuf::from("/test/dir");
    let policy = KeeperPolicy::ShortestPath;

    // 1. Create session
    let session_id = journal
        .create_session(&root, &policy)
        .expect("create session");
    assert_eq!(journal.active_session_id(), Some(session_id));

    let history = journal
        .fetch_session_history(session_id)
        .expect("fetch session");
    assert_eq!(history.session_id, session_id);
    assert_eq!(history.target_root, root);
    assert_eq!(history.status, SessionStatus::Active);
    assert!(history.ended_at.is_none());

    // 2. Complete session
    journal
        .complete_session(session_id)
        .expect("complete session");
    assert_eq!(journal.active_session_id(), None);

    let completed_history = journal
        .fetch_session_history(session_id)
        .expect("fetch session");
    assert_eq!(completed_history.status, SessionStatus::Completed);
    assert!(completed_history.ended_at.is_some());

    // 3. Completing already completed session must fail
    let err = journal
        .complete_session(session_id)
        .expect_err("double complete must fail");
    assert!(matches!(
        err,
        StorageError::InvalidSessionStateTransition { .. }
    ));
}

#[test]
fn test_plan_persistence_and_faithful_retrieval() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();

    let session_id = journal
        .create_session(&plan.root_dir, &plan.policy)
        .expect("create session");
    let plan_id = journal
        .save_plan_for_session(session_id, &plan)
        .expect("save plan");

    // Fetch plan details
    let details = journal.fetch_plan(plan_id).expect("fetch plan");
    assert_eq!(details.plan_id, plan_id);
    assert_eq!(details.session_id, session_id);
    assert_eq!(details.quarantine_root, plan.quarantine_dir);
    assert_eq!(details.summary, plan.summary);

    // Verify keepers
    assert_eq!(details.keepers.len(), 1);
    let keeper = &details.keepers[0];
    assert_eq!(keeper.path, plan.keepers[0].path);
    assert_eq!(keeper.size, 2048);
    assert_eq!(keeper.file_id, plan.keepers[0].file_id);
    assert_eq!(keeper.hard_link_aliases.len(), 1);
    assert_eq!(
        keeper.hard_link_aliases[0],
        plan.keepers[0].hard_link_aliases[0]
    );

    // Verify actions and preconditions
    assert_eq!(details.actions.len(), 2);
    for (i, action) in details.actions.iter().enumerate() {
        assert_eq!(action.action_index, i);
        assert_eq!(action.source_path, plan.actions[i].source_path);
        assert_eq!(action.quarantine_path, plan.actions[i].quarantine_path);
        assert_eq!(action.keeper_path, plan.actions[i].keeper_path);
        assert_eq!(action.state, ActionState::Planned);
        assert_eq!(
            action.preconditions.expected_size,
            plan.actions[i].preconditions.expected_size
        );
        assert_eq!(
            action.preconditions.expected_hash,
            plan.actions[i].preconditions.expected_hash
        );
        assert_eq!(
            action.preconditions.source_file_id,
            plan.actions[i].preconditions.source_file_id
        );
        assert_eq!(
            action.preconditions.keeper_file_id,
            plan.actions[i].preconditions.keeper_file_id
        );
    }
}

#[test]
fn test_action_state_machine_valid_transitions_and_audit_trail() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();
    let plan_id = journal.save_plan(&plan).expect("save plan");

    let actions = journal
        .list_actions_for_plan(plan_id)
        .expect("list actions");
    let action_id = actions[0].action_id;

    // Initial state: Planned
    assert_eq!(actions[0].state, ActionState::Planned);

    // 1. Planned -> Validating
    journal
        .record_action_transition(
            action_id,
            ActionState::Validating,
            Some("Beginning precondition revalidation"),
        )
        .expect("transition to Validating");

    let action = journal.get_action(action_id).expect("get action");
    assert_eq!(action.state, ActionState::Validating);

    // 2. Validating -> Executing
    journal
        .record_action_transition(
            action_id,
            ActionState::Executing,
            Some("Preconditions verified; performing quarantine move"),
        )
        .expect("transition to Executing");

    let action = journal.get_action(action_id).expect("get action");
    assert_eq!(action.state, ActionState::Executing);

    // 3. Executing -> Completed
    journal
        .record_action_transition(
            action_id,
            ActionState::Completed,
            Some("File safely quarantined"),
        )
        .expect("transition to Completed");

    let action = journal.get_action(action_id).expect("get action");
    assert_eq!(action.state, ActionState::Completed);

    // Verify chronological audit trail
    let events = journal
        .fetch_action_audit_trail(action_id)
        .expect("fetch audit trail");
    assert_eq!(
        events.len(),
        4,
        "Should have initial plan event + 3 transitions"
    );

    assert_eq!(events[0].from_state, None);
    assert_eq!(events[0].to_state, ActionState::Planned);

    assert_eq!(events[1].from_state, Some(ActionState::Planned));
    assert_eq!(events[1].to_state, ActionState::Validating);
    assert_eq!(
        events[1].details.as_deref(),
        Some("Beginning precondition revalidation")
    );

    assert_eq!(events[2].from_state, Some(ActionState::Validating));
    assert_eq!(events[2].to_state, ActionState::Executing);

    assert_eq!(events[3].from_state, Some(ActionState::Executing));
    assert_eq!(events[3].to_state, ActionState::Completed);
    assert_eq!(
        events[3].details.as_deref(),
        Some("File safely quarantined")
    );
}

#[test]
fn test_action_state_machine_invalid_transitions() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();
    let plan_id = journal.save_plan(&plan).expect("save plan");

    let actions = journal
        .list_actions_for_plan(plan_id)
        .expect("list actions");
    let action_id = actions[0].action_id;

    // Illegal jump: Planned -> Executing (skipping Validating)
    let err = journal
        .record_action_transition(action_id, ActionState::Executing, None)
        .expect_err("Planned -> Executing must be rejected");

    assert!(matches!(err, StorageError::InvalidStateTransition { .. }));

    // State in database must still be Planned
    let action = journal.get_action(action_id).expect("get action");
    assert_eq!(action.state, ActionState::Planned);
}

#[test]
fn test_undo_state_transition() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();
    let plan_id = journal.save_plan(&plan).expect("save plan");

    let actions = journal
        .list_actions_for_plan(plan_id)
        .expect("list actions");
    let action_id = actions[0].action_id;

    // Progress to Completed
    journal
        .record_action_transition(action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(action_id, ActionState::Executing, None)
        .unwrap();
    journal
        .record_action_transition(action_id, ActionState::Completed, None)
        .unwrap();

    // Revert (Undo)
    journal
        .record_action_transition(
            action_id,
            ActionState::Reverted,
            Some("Restored to original location during undo"),
        )
        .expect("Completed -> Reverted must succeed");

    let action = journal.get_action(action_id).expect("get action");
    assert_eq!(action.state, ActionState::Reverted);

    let events = journal.fetch_action_audit_trail(action_id).unwrap();
    assert_eq!(events.last().unwrap().to_state, ActionState::Reverted);
}

#[test]
fn test_crash_simulation_incomplete_actions() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();
    let plan_id = journal.save_plan(&plan).expect("save plan");

    let actions = journal
        .list_actions_for_plan(plan_id)
        .expect("list actions");
    let action0 = actions[0].action_id;
    let action1 = actions[1].action_id;

    // Action 0 completes
    journal
        .record_action_transition(action0, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(action0, ActionState::Executing, None)
        .unwrap();
    journal
        .record_action_transition(action0, ActionState::Completed, None)
        .unwrap();

    // Action 1 begins execution and app is killed mid-flight
    journal
        .record_action_transition(action1, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(
            action1,
            ActionState::Executing,
            Some("In-flight mutation when crash occurred"),
        )
        .unwrap();

    // Incomplete actions query should identify Action 1 immediately
    let incomplete = journal
        .get_incomplete_actions()
        .expect("query incomplete actions");
    assert_eq!(incomplete.len(), 1);
    assert_eq!(incomplete[0].action_id, action1);
    assert_eq!(incomplete[0].state, ActionState::Executing);
}

#[test]
fn test_startup_recovery_report_and_reconciliation() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();
    let plan_id = journal.save_plan(&plan).expect("save plan");

    let actions = journal.list_actions_for_plan(plan_id).unwrap();
    let action_id = actions[0].action_id;

    // Leave action in Executing
    journal
        .record_action_transition(action_id, ActionState::Validating, None)
        .unwrap();
    journal
        .record_action_transition(action_id, ActionState::Executing, None)
        .unwrap();

    // Startup inspection
    let report = inspect_startup_state(&journal).expect("inspect startup");
    assert!(!report.is_clean);
    assert_eq!(report.interrupted_sessions.len(), 1);
    assert_eq!(report.incomplete_actions.len(), 1);
    assert_eq!(report.incomplete_actions[0].action_id, action_id);

    // Reconcile
    let post_reconciliation = reconcile_interrupted_sessions(&mut journal).expect("reconciliation");
    assert!(post_reconciliation.is_clean);
    assert_eq!(post_reconciliation.incomplete_actions.len(), 0);

    // Verify session was marked Interrupted
    let session = journal
        .fetch_session_history(report.interrupted_sessions[0].session_id)
        .unwrap();
    assert_eq!(session.status, SessionStatus::Interrupted);

    // Verify action was marked Failed with diagnostic explanation in audit log
    let action = journal.get_action(action_id).unwrap();
    assert_eq!(action.state, ActionState::Failed);

    let events = journal.fetch_action_audit_trail(action_id).unwrap();
    let last_event = events.last().unwrap();
    assert_eq!(last_event.to_state, ActionState::Failed);
    assert!(last_event
        .details
        .as_ref()
        .unwrap()
        .contains("destination verification required"));
}

#[test]
fn test_foreign_key_cascades() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();
    let plan_id = journal.save_plan(&plan).expect("save plan");

    let actions = journal.list_actions_for_plan(plan_id).unwrap();
    assert_eq!(actions.len(), 2);

    let conn = journal.connection();

    // Verify records exist
    let action_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM actions;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(action_count, 2);

    let precond_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM preconditions;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(precond_count, 2);

    // Delete plan row directly: cascade should delete actions, preconditions, keepers, events
    conn.execute("DELETE FROM plans WHERE plan_id = ?", [plan_id.to_string()])
        .unwrap();

    let post_action_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM actions;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        post_action_count, 0,
        "Actions must cascade on plan deletion"
    );

    let post_precond_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM preconditions;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        post_precond_count, 0,
        "Preconditions must cascade on action deletion"
    );

    let post_events_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM action_events;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        post_events_count, 0,
        "Events must cascade on action deletion"
    );
}

#[test]
fn test_transactional_rollback_on_failure() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();

    // Attempting to save plan for a non-existent session must fail cleanly
    let fake_session_id = reclaim_storage::SessionId::new();
    let result = journal.save_plan_for_session(fake_session_id, &plan);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        StorageError::SessionNotFound(_)
    ));

    // Verify database has zero plans, zero keepers, zero actions
    let conn = journal.connection();
    let plan_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM plans;", [], |r| r.get(0))
        .unwrap();
    let action_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM actions;", [], |r| r.get(0))
        .unwrap();
    let keeper_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM keepers;", [], |r| r.get(0))
        .unwrap();

    assert_eq!(plan_count, 0, "No plan row should be persisted");
    assert_eq!(action_count, 0, "No action row should be persisted");
    assert_eq!(keeper_count, 0, "No keeper row should be persisted");
}

#[test]
fn test_save_plan_implicit_session() {
    let mut journal = ActionJournal::open_in_memory().expect("open journal");
    let plan = create_mock_plan();

    // No session was created before save_plan
    assert_eq!(journal.active_session_id(), None);

    let plan_id = journal.save_plan(&plan).expect("save plan");
    assert!(journal.active_session_id().is_some());

    let plan_details = journal.fetch_plan(plan_id).expect("fetch plan");
    assert_eq!(Some(plan_details.session_id), journal.active_session_id());
}
