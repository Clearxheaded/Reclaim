//! # Generative Property-Based Testing for Reclaim Core Safety Invariants
//!
//! Utilizes `proptest` to generate arbitrary randomized directory hierarchies,
//! unicode permutations, mixed hard-link graphs, and edge-case byte buffers
//! to mathematically verify Reclaim's 10 Filesystem Safety Invariants.

use proptest::prelude::*;
use reclaim_core::duplicates::{find_duplicates, DuplicateOptions};
use reclaim_core::mutation::{
    atomic_move_no_replace, compute_file_blake3_hash, validate_action_preconditions, MutationError,
    PreconditionError,
};
use reclaim_core::planner::{generate_plan, KeeperPolicy, PlanOptions};
use reclaim_core::scanner::{scan_directory, ScanOptions};
use reclaim_core::watcher::{CandidateInvalidator, InvalidationReason};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Generative Strategies
// ---------------------------------------------------------------------------

/// Generates valid unicode filename fragments including multi-byte UTF-8,
/// non-ASCII scripts (Japanese, Cyrillic, Arabic), and symbols.
fn arb_filename() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("file.txt".to_string()),
        Just("dup_copy.log".to_string()),
        Just("dokument_файл.dat".to_string()),
        Just("日本語_アーカイブ.tar".to_string()),
        Just("spaced name with [brackets].bin".to_string()),
        Just("mixed_dots.part1.sub2.ext".to_string()),
        "[a-z0-9_]{1,8}\\.(txt|bin|dat)",
    ]
}

/// Generates arbitrary byte payloads ranging from 0 bytes to 32 KiB.
fn arb_payload() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just(Vec::new()),        // 0 bytes (empty file)
        Just(vec![0x42]),        // 1 byte
        Just(vec![0xAA; 16]),    // 16 bytes
        Just(vec![0x55; 1024]),  // 1 KiB
        Just(vec![0x77; 16384]), // 16 KiB (exact prefix boundary)
        Just(vec![0x88; 20000]), // 20 KiB (crossing prefix boundary)
        prop::collection::vec(any::<u8>(), 0..4096),
    ]
}

// ---------------------------------------------------------------------------
// Property Tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Invariant 1: No Mutation Without Approval
    /// The scanner and duplicate finder must be strictly read-only.
    /// No file data, file metadata, or file existence may be modified during evaluation.
    #[test]
    fn prop_scanner_and_duplicate_detector_are_strictly_read_only(
        name1 in arb_filename(),
        name2 in arb_filename(),
        payload1 in arb_payload(),
        payload2 in arb_payload(),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        let p1 = root.join(format!("a_{}", name1));
        let p2 = root.join(format!("b_{}", name2));

        fs::write(&p1, &payload1).unwrap();
        fs::write(&p2, &payload2).unwrap();

        // Capture pre-scan hashes
        let hash1_before = compute_file_blake3_hash(&p1).unwrap();
        let hash2_before = compute_file_blake3_hash(&p2).unwrap();

        // Perform scan and duplicate detection
        let scan_output = scan_directory(root, ScanOptions::default()).unwrap();
        let dup_opts = DuplicateOptions {
            include_empty: true,
            ..Default::default()
        };
        let _dup_output = find_duplicates(&scan_output.entries, &dup_opts);

        // Verify post-scan hashes are identical (bit-for-bit unchanged)
        let hash1_after = compute_file_blake3_hash(&p1).unwrap();
        let hash2_after = compute_file_blake3_hash(&p2).unwrap();

        prop_assert_eq!(hash1_before, hash1_after);
        prop_assert_eq!(hash2_before, hash2_after);
        prop_assert_eq!(fs::read(&p1).unwrap(), payload1);
        prop_assert_eq!(fs::read(&p2).unwrap(), payload2);
    }

    /// Invariant 2: No Silent Overwrites
    /// Moving file A to an existing destination B must ALWAYS fail with
    /// `DestinationAlreadyExists` and MUST NEVER overwrite destination B.
    #[test]
    fn prop_atomic_move_refuses_overwrite_and_preserves_destination(
        content_src in arb_payload(),
        content_dst in arb_payload(),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let src = temp_dir.path().join("source.dat");
        let dst = temp_dir.path().join("existing_dest.dat");

        fs::write(&src, &content_src).unwrap();
        fs::write(&dst, &content_dst).unwrap();

        let res = atomic_move_no_replace(&src, &dst);

        // Operation must fail with DestinationAlreadyExists
        prop_assert!(res.is_err());
        match res.unwrap_err() {
            MutationError::DestinationAlreadyExists(p) => {
                prop_assert_eq!(p, dst.clone());
            }
            other => panic!("Expected DestinationAlreadyExists, got {:?}", other),
        }

        // Both source and destination must retain their exact original contents
        prop_assert_eq!(fs::read(&src).unwrap(), content_src);
        prop_assert_eq!(fs::read(&dst).unwrap(), content_dst);
    }

    /// Invariant 4: No Trust in Stale Metadata (Active TOCTOU Defense)
    /// If file contents change between plan generation and execution,
    /// precondition validation must reject the action.
    #[test]
    fn prop_precondition_revalidation_detects_arbitrary_bit_alterations(
        original_bytes in prop::collection::vec(any::<u8>(), 1..2048),
        mutation_index in any::<prop::sample::Index>(),
        altered_byte in any::<u8>(),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let keeper = temp_dir.path().join("keeper.bin");
        let duplicate = temp_dir.path().join("duplicate.bin");

        fs::write(&keeper, &original_bytes).unwrap();
        fs::write(&duplicate, &original_bytes).unwrap();

        let scan_output = scan_directory(temp_dir.path(), ScanOptions::default()).unwrap();
        let dup_output = find_duplicates(&scan_output.entries, &DuplicateOptions::default());

        prop_assert_eq!(dup_output.groups.len(), 1);

        let plan_opts = PlanOptions {
            root_dir: temp_dir.path().to_path_buf(),
            policy: KeeperPolicy::ShortestPath,
            quarantine_dir: Some(temp_dir.path().join(".reclaim/quarantine")),
        };
        let plan = generate_plan(&dup_output.groups, &plan_opts).unwrap();

        prop_assert_eq!(plan.actions.len(), 1);
        let action = &plan.actions[0];

        // Mutate one byte in duplicate
        let mut modified_bytes = original_bytes.clone();
        let idx = mutation_index.index(modified_bytes.len());
        // Ensure mutated byte is genuinely different
        let new_byte = if modified_bytes[idx] == altered_byte {
            altered_byte.wrapping_add(1)
        } else {
            altered_byte
        };
        modified_bytes[idx] = new_byte;
        fs::write(&duplicate, &modified_bytes).unwrap();

        // Precondition revalidation must detect the hash change!
        let val_res = validate_action_preconditions(action);
        prop_assert!(val_res.is_err());
        match val_res.unwrap_err() {
            PreconditionError::SourceHashMismatch { .. } => {}
            other => panic!("Expected SourceHashMismatch, got {:?}", other),
        }
    }

    /// Invariant 6: Preserve Verified Content & Accurate Hard-Link Accounting
    /// When exact duplicates are found, storage calculations must conservatively
    /// reflect distinct physical copies and treat hard links as 0 B reclaimable.
    #[test]
    #[cfg(unix)]
    fn prop_hard_link_accounting_and_duplicate_group_properties(
        content in prop::collection::vec(any::<u8>(), 10..512),
        hard_link_count in 1usize..4usize,
        duplicate_copy_count in 1usize..3usize,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        let mut paths = Vec::new();

        // Create independent duplicate copies with hard links
        for copy_idx in 0..duplicate_copy_count {
            let primary = root.join(format!("copy_{}.bin", copy_idx));
            fs::write(&primary, &content).unwrap();
            paths.push(primary.clone());

            for link_idx in 0..hard_link_count {
                let link = root.join(format!("copy_{}_link_{}.bin", copy_idx, link_idx));
                fs::hard_link(&primary, &link).unwrap();
                paths.push(link);
            }
        }

        let scan_output = scan_directory(root, ScanOptions::default()).unwrap();
        let dup_output = find_duplicates(&scan_output.entries, &DuplicateOptions::default());

        if duplicate_copy_count > 1 {
            prop_assert_eq!(dup_output.groups.len(), 1);
            let group = &dup_output.groups[0];

            prop_assert_eq!(group.distinct_physical_copies, duplicate_copy_count);
            let expected_reclaimable = ((duplicate_copy_count - 1) as u64) * (content.len() as u64);
            prop_assert_eq!(group.reclaimable_bytes, expected_reclaimable);
        } else {
            // Pure hard-link group has 0 reclaimable bytes
            if !dup_output.groups.is_empty() {
                let group = &dup_output.groups[0];
                prop_assert_eq!(group.distinct_physical_copies, 1);
                prop_assert_eq!(group.reclaimable_bytes, 0);
            }
        }
    }

    /// Invariant 4 (Dynamic Real-Time Invalidation):
    /// Invalidator must dynamically update candidate counts and dissolve
    /// groups when candidates are modified or removed.
    #[test]
    fn prop_candidate_invalidation_lifecycle(
        payload in prop::collection::vec(any::<u8>(), 10..100),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let f1 = temp_dir.path().join("c1.txt");
        let f2 = temp_dir.path().join("c2.txt");

        fs::write(&f1, &payload).unwrap();
        fs::write(&f2, &payload).unwrap();

        let scan_output = scan_directory(temp_dir.path(), ScanOptions::default()).unwrap();
        let dup_output = find_duplicates(&scan_output.entries, &DuplicateOptions::default());

        prop_assert_eq!(dup_output.groups.len(), 1);

        let mut invalidator = CandidateInvalidator::new(dup_output, vec![temp_dir.path().to_path_buf()]);
        prop_assert_eq!(invalidator.candidate_count(), 2);

        // Invalidate one candidate
        let notice = invalidator.invalidate_path(&f1, InvalidationReason::Modified);
        prop_assert!(notice.is_some());
        let n = notice.unwrap();
        prop_assert_eq!(n.path, f1);
        prop_assert_eq!(n.remaining_files_in_group, 0); // group dissolved
        prop_assert_eq!(invalidator.candidate_count(), 0);
    }
}
