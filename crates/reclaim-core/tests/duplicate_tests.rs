use reclaim_core::{
    find_duplicates, scan_directory, CancellationToken, DuplicateOptions, DuplicateProgress,
    DuplicateStage, ScanOptions,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("reclaim_dedup_{}_{}", name, unique));
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
fn test_unique_sizes_filtered_without_hashing() {
    let fixture = TestDir::new("unique_sizes");
    fixture.create_file("file1.bin", &[1; 100]);
    fixture.create_file("file2.bin", &[2; 200]);
    fixture.create_file("file3.bin", &[3; 300]);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");
    assert_eq!(scan_out.entries.len(), 3);

    let dedup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert!(dedup_out.groups.is_empty());
    assert_eq!(dedup_out.summary.total_files_evaluated, 3);
    assert_eq!(dedup_out.summary.size_matched_candidates, 0);
    assert_eq!(dedup_out.summary.prefix_matched_candidates, 0);
    assert_eq!(dedup_out.summary.total_reclaimable_bytes, 0);
}

#[test]
fn test_same_size_different_prefix_eliminated_stage1() {
    let fixture = TestDir::new("diff_prefix");
    // Two 20 KB files that differ in their first byte
    let mut data1 = vec![0xAA; 20_000];
    let mut data2 = vec![0xBB; 20_000];
    data1[0] = 0x01;
    data2[0] = 0x02;

    fixture.create_file("a.dat", &data1);
    fixture.create_file("b.dat", &data2);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");
    assert_eq!(scan_out.entries.len(), 2);

    let dedup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert!(dedup_out.groups.is_empty());
    assert_eq!(dedup_out.summary.size_matched_candidates, 2);
    // Stage 1 prefix hashing should eliminate them, leaving 0 prefix-matched candidates
    assert_eq!(dedup_out.summary.prefix_matched_candidates, 0);
    assert_eq!(dedup_out.summary.total_reclaimable_bytes, 0);
}

#[test]
fn test_same_prefix_different_suffix_eliminated_stage2() {
    let fixture = TestDir::new("diff_suffix");
    // Two 32 KB files sharing identical first 16 KB (the default prefix size)
    // but differing in their second 16 KB
    let prefix = vec![0x42; 16 * 1024];

    let mut data1 = prefix.clone();
    data1.extend_from_slice(&vec![0xAA; 16 * 1024]);

    let mut data2 = prefix;
    data2.extend_from_slice(&vec![0xBB; 16 * 1024]);

    fixture.create_file("file1.dat", &data1);
    fixture.create_file("file2.dat", &data2);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");
    assert_eq!(scan_out.entries.len(), 2);

    let dedup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    // Both passed size check and prefix check, but Stage 2 full hashing eliminated them
    assert!(dedup_out.groups.is_empty());
    assert_eq!(dedup_out.summary.size_matched_candidates, 2);
    assert_eq!(dedup_out.summary.prefix_matched_candidates, 2);
    assert_eq!(dedup_out.summary.duplicate_groups_count, 0);
    assert_eq!(dedup_out.summary.total_reclaimable_bytes, 0);
}

#[test]
fn test_exact_duplicates_in_subdirectories() {
    let fixture = TestDir::new("exact_duplicates");
    let content = b"Duplicate content for testing Reclaim duplicate detection!";

    fixture.create_file("root.txt", content);
    fixture.create_file("sub/nested/copy1.txt", content);
    fixture.create_file("sub/other/copy2.txt", content);
    // Add an unrelated file of different size
    fixture.create_file("unrelated.txt", b"something else completely");

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");
    assert_eq!(scan_out.entries.len(), 4);

    let dedup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert_eq!(dedup_out.groups.len(), 1);
    let group = &dedup_out.groups[0];
    assert_eq!(group.file_count(), 3);
    assert_eq!(group.distinct_physical_copies, 3);
    assert_eq!(group.file_size, content.len() as u64);
    assert_eq!(group.reclaimable_bytes, 2 * content.len() as u64);
    assert_eq!(group.hash, blake3::hash(content));
    assert!(!group.has_hard_links());

    assert_eq!(dedup_out.summary.duplicate_groups_count, 1);
    assert_eq!(dedup_out.summary.duplicate_files_count, 3);
    assert_eq!(dedup_out.summary.distinct_physical_copies, 3);
    assert_eq!(dedup_out.summary.hard_link_aliases_count, 0);
    assert_eq!(
        dedup_out.summary.total_reclaimable_bytes,
        2 * content.len() as u64
    );
}

#[test]
fn test_large_file_streaming_exact_duplicates() {
    let fixture = TestDir::new("large_streaming");
    // 128 KiB file, requiring multiple 64 KiB buffer reads during Stage 2
    let mut large_data = Vec::with_capacity(128 * 1024);
    for i in 0..(128 * 1024) {
        large_data.push((i % 251) as u8);
    }

    fixture.create_file("large_a.bin", &large_data);
    fixture.create_file("nested/large_b.bin", &large_data);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");
    assert_eq!(scan_out.entries.len(), 2);

    let dedup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert_eq!(dedup_out.groups.len(), 1);
    let group = &dedup_out.groups[0];
    assert_eq!(group.file_count(), 2);
    assert_eq!(group.file_size, 128 * 1024);
    assert_eq!(group.reclaimable_bytes, 128 * 1024);
    assert_eq!(group.hash, blake3::hash(&large_data));
}

#[test]
fn test_empty_files_handling() {
    let fixture = TestDir::new("empty_files");
    fixture.create_file("empty1.txt", b"");
    fixture.create_file("empty2.txt", b"");
    fixture.create_file("sub/empty3.txt", b"");

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");
    assert_eq!(scan_out.entries.len(), 3);

    // 1. By default, empty files are ignored
    let default_opts = DuplicateOptions::default();
    let dedup_out = find_duplicates(&scan_out.entries, &default_opts);
    assert!(dedup_out.groups.is_empty());
    assert_eq!(dedup_out.summary.empty_files_skipped, 3);
    assert_eq!(dedup_out.summary.total_reclaimable_bytes, 0);

    // 2. When include_empty: true, empty files are grouped, but reclaimable_bytes is 0
    let include_opts = DuplicateOptions {
        include_empty: true,
        ..Default::default()
    };
    let dedup_empty = find_duplicates(&scan_out.entries, &include_opts);
    assert_eq!(dedup_empty.groups.len(), 1);
    let group = &dedup_empty.groups[0];
    assert_eq!(group.file_count(), 3);
    assert_eq!(group.file_size, 0);
    assert_eq!(group.reclaimable_bytes, 0);
    assert_eq!(group.hash, blake3::hash(b""));
    assert_eq!(dedup_empty.summary.total_reclaimable_bytes, 0);
}

#[test]
#[cfg(unix)]
fn test_hard_links_and_distinct_copies() {
    let fixture = TestDir::new("hard_links");
    let content = b"Physical storage blocks shared between hard link entries.";
    let file1 = fixture.create_file("file1.dat", content);
    let hardlink1 = fixture.path().join("hardlink1.dat");

    // Attempt to create a hard link
    let hard_link_created = fs::hard_link(&file1, &hardlink1).is_ok();
    if !hard_link_created {
        // Filesystem does not support hard links; skip gracefully
        return;
    }

    // Create a separate physical file with identical content
    fixture.create_file("independent_copy.dat", content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");
    assert_eq!(scan_out.entries.len(), 3);

    let dedup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert_eq!(dedup_out.groups.len(), 1);
    let group = &dedup_out.groups[0];

    // Total files = 3
    assert_eq!(group.file_count(), 3);
    // Distinct physical copies = 2 (file1 + hardlink1 share one inode, independent_copy has another)
    assert_eq!(group.distinct_physical_copies, 2);
    // Hard links detected!
    assert!(group.has_hard_links());
    assert!(!group.is_pure_hard_links());

    // Crucial check: Reclaimable storage is (2 distinct copies - 1) * size = 1 * size
    // NOT (3 - 1) * size = 2 * size
    assert_eq!(group.reclaimable_bytes, content.len() as u64);
    assert_eq!(
        dedup_out.summary.total_reclaimable_bytes,
        content.len() as u64
    );
    assert_eq!(dedup_out.summary.hard_link_aliases_count, 1);
}

#[test]
#[cfg(unix)]
fn test_pure_hard_links_zero_reclaimable_bytes() {
    let fixture = TestDir::new("pure_hard_links");
    let content = b"Pure hard link test - no independent duplicate copies.";
    let file1 = fixture.create_file("source.dat", content);
    let link1 = fixture.path().join("alias1.dat");
    let link2 = fixture.path().join("alias2.dat");

    if fs::hard_link(&file1, &link1).is_err() || fs::hard_link(&file1, &link2).is_err() {
        return;
    }

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");
    assert_eq!(scan_out.entries.len(), 3);

    let dedup_out = find_duplicates(&scan_out.entries, &DuplicateOptions::default());

    assert_eq!(dedup_out.groups.len(), 1);
    let group = &dedup_out.groups[0];

    assert_eq!(group.file_count(), 3);
    // All 3 point to the same physical storage blocks!
    assert_eq!(group.distinct_physical_copies, 1);
    assert!(group.has_hard_links());
    assert!(group.is_pure_hard_links());

    // Storage savings MUST be 0, because deleting any hard link frees 0 disk blocks!
    assert_eq!(group.reclaimable_bytes, 0);
    assert_eq!(dedup_out.summary.total_reclaimable_bytes, 0);
    assert_eq!(dedup_out.summary.hard_link_aliases_count, 2);
}

#[test]
fn test_cancellation_during_hashing() {
    let fixture = TestDir::new("cancellation");
    let content = vec![0x55; 10_000];
    fixture.create_file("f1.bin", &content);
    fixture.create_file("f2.bin", &content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");

    let cancel_token = CancellationToken::new();
    cancel_token.cancel(); // Pre-cancel

    let options = DuplicateOptions {
        cancellation_token: Some(cancel_token),
        ..Default::default()
    };

    let dedup_out = find_duplicates(&scan_out.entries, &options);
    assert!(dedup_out.summary.was_cancelled);
}

#[test]
fn test_progress_callback_reporting() {
    let fixture = TestDir::new("progress");
    let content = vec![0x77; 20_000];
    fixture.create_file("p1.bin", &content);
    fixture.create_file("p2.bin", &content);

    let scan_out = scan_directory(fixture.path(), ScanOptions::default()).expect("scan failed");

    let prefix_events = Arc::new(AtomicUsize::new(0));
    let full_events = Arc::new(AtomicUsize::new(0));

    let prefix_counter = Arc::clone(&prefix_events);
    let full_counter = Arc::clone(&full_events);

    let options = DuplicateOptions {
        progress_callback: Some(Box::new(move |prog: &DuplicateProgress| {
            match &prog.stage {
                DuplicateStage::PrefixHashing { .. } => {
                    prefix_counter.fetch_add(1, Ordering::Relaxed);
                }
                DuplicateStage::FullHashing { .. } => {
                    full_counter.fetch_add(1, Ordering::Relaxed);
                }
                DuplicateStage::GroupingBySize => {}
            }
        })),
        ..Default::default()
    };

    let dedup_out = find_duplicates(&scan_out.entries, &options);
    assert_eq!(dedup_out.groups.len(), 1);

    // Prefix hashing should have been called for candidates
    assert!(prefix_events.load(Ordering::Relaxed) >= 2);
    // Full hashing should have been called for candidates > 16 KiB
    assert!(full_events.load(Ordering::Relaxed) >= 2);
}
