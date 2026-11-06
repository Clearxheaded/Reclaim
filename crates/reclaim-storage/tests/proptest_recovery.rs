//! # Generative Property-Based Testing for Reclaim Storage & Recovery
//!
//! Utilizes `proptest` to generate arbitrary simulated crash states, interrupted sessions,
//! destination collisions, and payload permutations to verify Reclaim's crash recovery
//! idempotency (Invariant 8) and honest undo guarantees (Invariant 7).

use proptest::prelude::*;
use reclaim_core::duplicates::{find_duplicates, DuplicateOptions};
use reclaim_core::planner::{generate_plan, CleanupPlan, KeeperPolicy, PlanOptions};
use reclaim_core::scanner::{scan_directory, ScanOptions};
use reclaim_storage::recovery::{inspect_startup_state, reconcile_startup_state};
use reclaim_storage::undo::{SafeReverter, UndoConflictReason, UndoOptions};
use reclaim_storage::{ActionJournal, ActionState, ExecutionOptions, SafeExecutor};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn create_plan(root: &Path, quarantine_dir: &Path) -> CleanupPlan {
    let scan_out = scan_directory(root, ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());
    let opts = PlanOptions {
        root_dir: root.to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: Some(quarantine_dir.to_path_buf()),
    };
    generate_plan(&dup_out.groups, &opts).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(25))]

    /// Invariant 8: Crash Recovery & Reconciliation Idempotency
    /// No matter what state a crash leaves an action in (Completed, Executing with file moved,
    /// Executing with file unmoved, or Pending), reconciliation must resolve cleanly and
    /// subsequent reconciliations must be strictly idempotent (0-op).
    #[test]
    fn prop_reconciliation_is_strictly_idempotent(
        content in prop::collection::vec(any::<u8>(), 20..200),
        crash_scenario in 0u8..3u8,
    ) {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let dir_a = root.join("dir_a");
        let dir_b = root.join("dir_b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        let f_orig = dir_a.join("keeper.txt");
        let f_dup = dir_b.join("dup.txt");

        fs::write(&f_orig, &content).unwrap();
        fs::write(&f_dup, &content).unwrap();

        let q_dir = root.join(".reclaim/quarantine");
        let plan = create_plan(root, &q_dir);
        prop_assert_eq!(plan.actions.len(), 1);

        let temp_db = TempDir::new().unwrap();
        let db_path = temp_db.path().join("journal.db");
        let mut journal = ActionJournal::open(&db_path).unwrap();

        let plan_id = journal.save_plan(&plan).unwrap();
        let actions = journal.list_actions_for_plan(plan_id).unwrap();
        let action = &actions[0];

        match crash_scenario {
            0 => {
                // Scenario 0: Executing, but file was already moved to quarantine on disk
                journal.record_action_transition(action.action_id, ActionState::Validating, None).unwrap();
                journal.record_action_transition(action.action_id, ActionState::Executing, None).unwrap();
                fs::create_dir_all(action.quarantine_path.parent().unwrap()).unwrap();
                fs::rename(&f_dup, &action.quarantine_path).unwrap();
            }
            1 => {
                // Scenario 1: Executing, but file remained at source (crash before move)
                journal.record_action_transition(action.action_id, ActionState::Validating, None).unwrap();
                journal.record_action_transition(action.action_id, ActionState::Executing, None).unwrap();
            }
            _ => {
                // Scenario 2: Pending (crash before execution began)
            }
        }

        // Pass 1: Initial startup reconciliation
        let report1 = reconcile_startup_state(&mut journal).unwrap();
        prop_assert!(!report1.is_clean);

        // Pass 2: Immediate second reconciliation must be completely idempotent (0-op)
        let report2 = reconcile_startup_state(&mut journal).unwrap();
        prop_assert!(report2.is_clean);
        prop_assert_eq!(report2.actions_checked, 0);
        prop_assert_eq!(report2.sessions_interrupted, 0);

        let startup = inspect_startup_state(&journal).unwrap();
        prop_assert!(startup.is_clean);
    }

    /// Invariant 7: Honest Undo Guarantees (Destination Collision Defense)
    /// If an unrelated file is placed at the original source path, undo MUST fail
    /// closed with `DestinationOccupied` and MUST NOT overwrite the occupying file!
    #[test]
    fn prop_undo_rejects_destination_collision_and_preserves_occupier(
        original_bytes in prop::collection::vec(any::<u8>(), 10..100),
        occupier_bytes in prop::collection::vec(any::<u8>(), 10..100),
    ) {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let dir_a = root.join("dir_a");
        let dir_b = root.join("dir_b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        let f_keeper = dir_a.join("keeper.txt");
        let f_dup = dir_b.join("dup.txt");

        fs::write(&f_keeper, &original_bytes).unwrap();
        fs::write(&f_dup, &original_bytes).unwrap();

        let q_dir = root.join(".reclaim/quarantine");
        let plan = create_plan(root, &q_dir);

        let temp_db = TempDir::new().unwrap();
        let db_path = temp_db.path().join("journal.db");
        let mut journal = ActionJournal::open(&db_path).unwrap();

        let orig_source = plan.actions[0].source_path.clone();

        let mut executor = SafeExecutor::new(ExecutionOptions::default());
        let exec_report = executor.execute_plan(&mut journal, &plan).unwrap();
        prop_assert_eq!(exec_report.completed_actions, 1);

        // Invalidate original source location by placing an occupying file
        fs::write(&orig_source, &occupier_bytes).unwrap();

        // Attempt undo on the session
        let reverter = SafeReverter::new(UndoOptions::default());
        let undo_report = reverter
            .revert_session(&mut journal, exec_report.session_id)
            .unwrap();

        // Undo must report conflict and refuse overwrite!
        prop_assert_eq!(undo_report.actions_restored, 0);
        prop_assert_eq!(undo_report.actions_conflicted, 1);
        prop_assert_eq!(undo_report.conflicts.len(), 1);

        match &undo_report.conflicts[0].reason {
            UndoConflictReason::DestinationOccupied { .. } => {}
            other => panic!("Expected DestinationOccupied conflict, got: {:?}", other),
        }

        // Occupier must remain 100% intact
        prop_assert_eq!(fs::read(&orig_source).unwrap(), occupier_bytes);
    }
}
