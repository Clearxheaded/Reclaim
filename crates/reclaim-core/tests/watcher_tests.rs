use reclaim_core::duplicates::{find_duplicates, DuplicateOptions};
use reclaim_core::scanner::{scan_directory, ScanOptions};
use reclaim_core::watcher::{ActiveFsWatcher, CandidateInvalidator, InvalidationReason};
use std::fs;
use std::thread::sleep;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn test_invalidator_unit_lifecycle() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    let f1 = root.join("file1.txt");
    let f2 = root.join("file2.txt");
    let f3 = root.join("file3.txt");

    fs::write(&f1, b"common payload 100").unwrap();
    fs::write(&f2, b"common payload 100").unwrap();
    fs::write(&f3, b"common payload 100").unwrap();

    let scan_out = scan_directory(root, ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert_eq!(dup_out.groups.len(), 1);
    assert_eq!(dup_out.groups[0].entries.len(), 3);
    assert_eq!(dup_out.groups[0].reclaimable_bytes, 18 * 2);

    let mut invalidator = CandidateInvalidator::new(dup_out, vec![root.to_path_buf()]);
    assert_eq!(invalidator.candidate_count(), 3);
    assert!(invalidator.is_candidate(&f1));
    assert!(invalidator.is_candidate(&f2));
    assert!(invalidator.is_candidate(&f3));

    // Invalidate f3 by modification
    let notice = invalidator
        .invalidate_path(&f3, InvalidationReason::Modified)
        .expect("Notice should be produced");
    assert_eq!(notice.path, f3);
    assert_eq!(notice.reason, InvalidationReason::Modified);
    assert_eq!(notice.remaining_files_in_group, 2);
    assert_eq!(notice.new_reclaimable_bytes, 18);
    assert_eq!(invalidator.candidate_count(), 2);
    assert!(!invalidator.is_candidate(&f3));

    // Invalidate f2 by deletion -> leaves only 1 file in group -> group dissolves!
    let notice2 = invalidator
        .invalidate_path(&f2, InvalidationReason::Removed)
        .expect("Notice should be produced");
    assert_eq!(notice2.path, f2);
    assert_eq!(notice2.remaining_files_in_group, 0);
    assert_eq!(notice2.new_reclaimable_bytes, 0);
    assert_eq!(invalidator.candidate_count(), 0);

    let current = invalidator.current_output();
    assert_eq!(current.groups.len(), 0);
    assert_eq!(current.summary.total_reclaimable_bytes, 0);
}

#[test]
fn test_invalidator_ancestor_directory_removal() {
    let temp_dir = TempDir::new().unwrap();
    let sub = temp_dir.path().join("subfolder");
    fs::create_dir(&sub).unwrap();

    let f1 = temp_dir.path().join("f1.txt");
    let f2 = sub.join("f2.txt");

    fs::write(&f1, b"matching content").unwrap();
    fs::write(&f2, b"matching content").unwrap();

    let scan_out = scan_directory(temp_dir.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());
    assert_eq!(dup_out.groups.len(), 1);

    let mut invalidator = CandidateInvalidator::new(dup_out, vec![temp_dir.path().to_path_buf()]);
    assert_eq!(invalidator.candidate_count(), 2);

    // Invalidate parent folder `sub`
    let notice = invalidator
        .invalidate_path(&sub, InvalidationReason::Removed)
        .expect("Subfolder deletion should invalidate child candidate");

    assert_eq!(notice.reason, InvalidationReason::Removed);
    assert_eq!(invalidator.candidate_count(), 0);
    let current = invalidator.current_output();
    assert_eq!(current.groups.len(), 0);
}

#[test]
fn test_active_fs_watcher_real_time_event_streaming() {
    let temp_dir = TempDir::new().unwrap();
    let f1 = temp_dir.path().join("alpha.txt");
    let f2 = temp_dir.path().join("beta.txt");

    fs::write(&f1, b"exact duplicate payload").unwrap();
    fs::write(&f2, b"exact duplicate payload").unwrap();

    let scan_out = scan_directory(temp_dir.path(), ScanOptions::default()).unwrap();
    let dup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());
    assert_eq!(dup_out.groups.len(), 1);

    let watcher = ActiveFsWatcher::start(dup_out, vec![temp_dir.path().to_path_buf()]).unwrap();

    // Mutate f2 on disk
    sleep(Duration::from_millis(50));
    fs::write(&f2, b"mutated content by external process").unwrap();

    let f2_canon = fs::canonicalize(&f2).unwrap();

    // Poll for event
    let mut detected = false;
    for _ in 0..30 {
        sleep(Duration::from_millis(100));
        let notices = watcher.poll_invalidations();
        if !notices.is_empty() {
            detected = true;
            assert!(
                notices.iter().any(|n| n.path == f2 || n.path == f2_canon),
                "Expected invalidation for f2 ({:?}), got: {:?}",
                f2,
                notices
            );
            break;
        }
    }

    assert!(
        detected,
        "Active TOCTOU watcher should detect file mutation on disk in real time"
    );
}
