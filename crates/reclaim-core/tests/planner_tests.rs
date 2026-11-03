use reclaim_core::{
    find_duplicates, generate_plan, scan_directory, CleanupPlan, DuplicateOptions, KeeperPolicy,
    PlanError, PlanOptions, QuarantineAction, ScanOptions,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("reclaim_planner_{}_{}", name, unique));
        fs::create_dir_all(&path).expect("failed to create test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn create_file(&self, relative_path: impl AsRef<Path>, content: &[u8]) -> PathBuf {
        let p = self.path.join(relative_path);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("failed to create parent dir");
        }
        fs::write(&p, content).expect("failed to write test file");
        p
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_plan_immutability() {
    let fixture = TestDir::new("immutability");
    let content = b"Safety first: planner is strictly read-only and never modifies files!";
    let p1 = fixture.create_file("folder_a/doc.txt", content);
    let p2 = fixture.create_file("folder_b/nested/doc.txt", content);
    let p3 = fixture.create_file("unique.txt", b"Unique file contents");

    // Capture filesystem state before planning
    let mut before_state = HashMap::new();
    for p in &[&p1, &p2, &p3] {
        let meta = fs::metadata(p).expect("metadata before");
        let bytes = fs::read(p).expect("read before");
        before_state.insert((*p).clone(), (meta.len(), meta.modified().unwrap(), bytes));
    }

    let default_quarantine_path = fixture.path().join(".reclaim").join("quarantine");

    // Run scan, duplicate detection, and plan generation
    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());
    assert_eq!(dup_out.groups.len(), 1);

    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: None,
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).expect("plan generation failed");
    assert_eq!(plan.actions.len(), 1);

    // Verify filesystem state is 100% UNCHANGED
    for (p, (orig_len, orig_mtime, orig_bytes)) in before_state {
        assert!(
            p.exists(),
            "file '{}' must still exist on disk",
            p.display()
        );
        let meta = fs::metadata(&p).expect("metadata after");
        let bytes = fs::read(&p).expect("read after");

        assert_eq!(meta.len(), orig_len, "file size must remain unchanged");
        assert_eq!(
            meta.modified().unwrap(),
            orig_mtime,
            "mtime must remain unchanged"
        );
        assert_eq!(
            bytes, orig_bytes,
            "file bytes must be byte-for-byte identical"
        );
    }

    // Verify no quarantine directory was created on disk
    assert!(
        !default_quarantine_path.exists(),
        "Quarantine directory must NOT be created during planning phase!"
    );
}

#[test]
fn test_keeper_policy_shortest_path() {
    let fixture = TestDir::new("shortest_path");
    let content = b"Identical content across depths";
    let p_shallow = fixture.create_file("top.txt", content);
    let p_mid = fixture.create_file("level1/mid.txt", content);
    let p_deep = fixture.create_file("level1/level2/deep.txt", content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());
    assert_eq!(dup_out.groups.len(), 1);

    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: None,
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();
    assert_eq!(plan.keepers.len(), 1);
    assert_eq!(plan.keepers[0].path, p_shallow);

    assert_eq!(plan.actions.len(), 2);
    let action_sources: Vec<_> = plan.actions.iter().map(|a| &a.source_path).collect();
    assert!(action_sources.contains(&&p_mid));
    assert!(action_sources.contains(&&p_deep));
}

#[test]
fn test_keeper_policy_preferred_directory() {
    let fixture = TestDir::new("preferred_dir");
    let content = b"Preferred directory keeper test";
    // Canonical library path has depth 3
    let p_canonical = fixture.create_file("canonical/library/photo.jpg", content);
    // Temporary downloads path has depth 2 (shorter path length)
    let p_downloads = fixture.create_file("downloads/photo.jpg", content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());
    assert_eq!(dup_out.groups.len(), 1);

    let pref_dir = fixture.path().join("canonical");
    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::PreferredDirectory(pref_dir),
        quarantine_dir: None,
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();
    assert_eq!(plan.keepers.len(), 1);
    // Canonical must win even though downloads has a shorter path!
    assert_eq!(plan.keepers[0].path, p_canonical);
    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].source_path, p_downloads);
    assert_eq!(plan.actions[0].keeper_path, p_canonical);
}

#[test]
fn test_keeper_policy_oldest_and_newest_modified() {
    let fixture = TestDir::new("modified_policies");
    let content = b"Modified timestamp policy testing";
    let p1 = fixture.create_file("file1.txt", content);
    // Introduce a delay to ensure distinct mtime
    std::thread::sleep(Duration::from_millis(50));
    let p2 = fixture.create_file("file2.txt", content);

    let meta1 = fs::metadata(&p1).unwrap();
    let meta2 = fs::metadata(&p2).unwrap();
    assert!(meta1.modified().unwrap() < meta2.modified().unwrap());

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    // 1. Test OldestModified
    {
        let plan_opts = PlanOptions {
            root_dir: fixture.path().to_path_buf(),
            policy: KeeperPolicy::OldestModified,
            quarantine_dir: None,
        };
        let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();
        assert_eq!(
            plan.keepers[0].path, p1,
            "OldestModified should preserve p1"
        );
        assert_eq!(plan.actions[0].source_path, p2);
    }

    // 2. Test NewestModified
    {
        let plan_opts = PlanOptions {
            root_dir: fixture.path().to_path_buf(),
            policy: KeeperPolicy::NewestModified,
            quarantine_dir: None,
        };
        let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();
        assert_eq!(
            plan.keepers[0].path, p2,
            "NewestModified should preserve p2"
        );
        assert_eq!(plan.actions[0].source_path, p1);
    }
}

#[test]
fn test_precondition_capture() {
    let fixture = TestDir::new("preconditions");
    let content = b"Preconditions guard against TOCTOU race conditions!";
    // "a_keeper.bin" has fewer characters than "b_quarantined_copy.bin"
    let p_keeper = fixture.create_file("a_keeper.bin", content);
    let p_dup = fixture.create_file("b_quarantined_copy.bin", content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: None,
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();
    assert_eq!(plan.actions.len(), 1);

    let action = &plan.actions[0];
    let expected_hash = blake3::hash(content);

    // Verify all preconditions are recorded
    assert_eq!(action.preconditions.expected_size, content.len() as u64);
    assert_eq!(
        action.preconditions.expected_hash,
        expected_hash.to_hex().as_str()
    );
    assert_eq!(
        action.preconditions.expected_blake3_hash().unwrap(),
        expected_hash
    );

    assert!(action.preconditions.source_modified.is_some());
    assert!(action.preconditions.keeper_modified.is_some());

    #[cfg(unix)]
    {
        assert!(action.preconditions.source_file_id.is_some());
        assert!(action.preconditions.keeper_file_id.is_some());
    }

    assert_eq!(action.source_path, p_dup);
    assert_eq!(action.keeper_path, p_keeper);
}

#[test]
#[cfg(unix)]
fn test_pure_hard_link_group_zero_actions() {
    let fixture = TestDir::new("pure_hard_links");
    let content = b"Pure hard links share same physical blocks; 0 reclaimable bytes";
    let p1 = fixture.create_file("original.txt", content);
    let p2 = fixture.path().join("hardlink.txt");

    if fs::hard_link(&p1, &p2).is_err() {
        eprintln!("Filesystem does not support hard links in test dir; skipping test");
        return;
    }

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert_eq!(dup_out.groups.len(), 1);
    assert_eq!(dup_out.groups[0].distinct_physical_copies, 1);
    assert_eq!(dup_out.groups[0].reclaimable_bytes, 0);

    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: None,
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();

    // In a pure hard-link group, 0 actions should be proposed!
    assert_eq!(plan.actions.len(), 0);
    assert_eq!(plan.summary.planned_actions, 0);
    assert_eq!(plan.summary.total_reclaimable_bytes, 0);
    assert_eq!(plan.summary.files_left_untouched, 2);
    assert_eq!(plan.keepers.len(), 1);
    assert_eq!(plan.keepers[0].hard_link_aliases.len(), 1);
}

#[test]
#[cfg(unix)]
fn test_duplicate_physical_copies_with_hard_links() {
    let fixture = TestDir::new("copies_with_hard_links");
    let content = b"Mixture of physical copies and hard link aliases";
    let p1 = fixture.create_file("copy1.txt", content);
    let p1_link = fixture.path().join("copy1_link.txt");

    if fs::hard_link(&p1, &p1_link).is_err() {
        eprintln!("Filesystem does not support hard links; skipping test");
        return;
    }

    let p2 = fixture.create_file("copy2.txt", content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert_eq!(dup_out.groups.len(), 1);
    assert_eq!(dup_out.groups[0].distinct_physical_copies, 2);
    assert_eq!(dup_out.groups[0].entries.len(), 3);
    assert_eq!(dup_out.groups[0].reclaimable_bytes, content.len() as u64);

    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: None,
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();

    assert_eq!(plan.keepers.len(), 1);
    assert_eq!(plan.keepers[0].path, p1);
    // copy1_link is protected alongside copy1
    assert_eq!(plan.keepers[0].hard_link_aliases, vec![p1_link]);

    // Only copy2 is proposed for quarantine
    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].source_path, p2);
    assert_eq!(plan.actions[0].keeper_path, p1);
    assert_eq!(plan.summary.planned_actions, 1);
    assert_eq!(plan.summary.total_reclaimable_bytes, content.len() as u64);
    assert_eq!(plan.summary.files_left_untouched, 2);
}

#[test]
fn test_deterministic_lexical_tie_break() {
    let fixture = TestDir::new("lexical_ties");
    let content = b"Deterministic tie-breaking";
    let p_a = fixture.create_file("alpha.txt", content);
    let p_b = fixture.create_file("bravo.txt", content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: None,
    };

    // Run 10 times; alpha.txt must always be selected over bravo.txt
    for _ in 0..10 {
        let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();
        assert_eq!(plan.keepers[0].path, p_a);
        assert_eq!(plan.actions[0].source_path, p_b);
    }
}

#[test]
fn test_json_round_trip_serialization() {
    let fixture = TestDir::new("json_round_trip");
    let content = b"JSON serialization round-trip test";
    let _p1 = fixture.create_file("a/file1.txt", content);
    let _p2 = fixture.create_file("b/file2.txt", content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: None,
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();

    let json_pretty = plan.to_json_pretty().expect("failed to serialize JSON");
    assert!(json_pretty.contains("\"planned_actions\": 1"));
    assert!(json_pretty.contains("\"expected_hash\":"));

    let deserialized = CleanupPlan::from_json(&json_pretty).expect("failed to deserialize JSON");

    assert_eq!(plan.actions, deserialized.actions);
    assert_eq!(plan.keepers, deserialized.keepers);
    assert_eq!(plan.summary, deserialized.summary);
    assert_eq!(plan.policy, deserialized.policy);
    assert_eq!(plan.root_dir, deserialized.root_dir);
    assert_eq!(plan.quarantine_dir, deserialized.quarantine_dir);
}

#[test]
fn test_destination_collision_validation() {
    let dummy_preconditions = reclaim_core::ActionPreconditions {
        expected_size: 100,
        expected_hash: "0".repeat(64),
        source_modified: None,
        source_file_id: None,
        keeper_modified: None,
        keeper_file_id: None,
    };

    // 1. Colliding quarantine destinations
    let plan = CleanupPlan {
        created_at: SystemTime::now(),
        root_dir: PathBuf::from("/test"),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: PathBuf::from("/quarantine"),
        actions: vec![
            QuarantineAction {
                source_path: PathBuf::from("/test/a/doc.txt"),
                quarantine_path: PathBuf::from("/quarantine/doc.txt"),
                keeper_path: PathBuf::from("/test/keeper.txt"),
                preconditions: dummy_preconditions.clone(),
                rationale: "dup 1".into(),
            },
            QuarantineAction {
                source_path: PathBuf::from("/test/b/doc.txt"),
                quarantine_path: PathBuf::from("/quarantine/doc.txt"), // Collision!
                keeper_path: PathBuf::from("/test/keeper.txt"),
                preconditions: dummy_preconditions.clone(),
                rationale: "dup 2".into(),
            },
        ],
        keepers: vec![],
        summary: reclaim_core::PlanSummary {
            total_duplicate_files: 3,
            duplicate_groups_count: 1,
            planned_actions: 2,
            protected_keepers: 1,
            total_reclaimable_bytes: 200,
            files_left_untouched: 1,
        },
    };

    match plan.validate() {
        Err(PlanError::DestinationCollision { .. }) => {}
        other => panic!("expected DestinationCollision error, got {:?}", other),
    }

    // 2. Action targeting its own keeper
    let invalid_keeper_plan = CleanupPlan {
        created_at: SystemTime::now(),
        root_dir: PathBuf::from("/test"),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: PathBuf::from("/quarantine"),
        actions: vec![QuarantineAction {
            source_path: PathBuf::from("/test/keeper.txt"),
            quarantine_path: PathBuf::from("/quarantine/keeper.txt"),
            keeper_path: PathBuf::from("/test/keeper.txt"), // Keeper target!
            preconditions: dummy_preconditions,
            rationale: "invalid".into(),
        }],
        keepers: vec![],
        summary: reclaim_core::PlanSummary {
            total_duplicate_files: 1,
            duplicate_groups_count: 1,
            planned_actions: 1,
            protected_keepers: 0,
            total_reclaimable_bytes: 0,
            files_left_untouched: 0,
        },
    };

    match invalid_keeper_plan.validate() {
        Err(PlanError::KeeperTargetCollision { .. }) => {}
        other => panic!("expected KeeperTargetCollision error, got {:?}", other),
    }
}

#[test]
fn test_relative_quarantine_path_nesting() {
    let fixture = TestDir::new("nested_quarantine");
    let content = b"Nested quarantine directory structure preservation";
    let _p1 = fixture.create_file("top.txt", content);
    let p2 = fixture.create_file("sub/nested/deep_dup.txt", content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    let plan_opts = PlanOptions {
        root_dir: fixture.path().to_path_buf(),
        policy: KeeperPolicy::ShortestPath,
        quarantine_dir: None,
    };

    let plan = generate_plan(&dup_out.groups, &plan_opts).unwrap();
    assert_eq!(plan.actions.len(), 1);

    let action = &plan.actions[0];
    assert_eq!(action.source_path, p2);

    // Verify destination preserves sub/nested/deep_dup.txt under quarantine_dir
    let expected_dest = fixture
        .path()
        .join(".reclaim")
        .join("quarantine")
        .join("sub")
        .join("nested")
        .join("deep_dup.txt");
    assert_eq!(action.quarantine_path, expected_dest);
}
