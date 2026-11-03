#[cfg(target_os = "macos")]
use reclaim_core::mutation::cow::{
    deduplicate_via_cow, default_cow_deduplicator, ApfsCloneDeduplicator,
};
use reclaim_core::mutation::cow::{CowError, ExtentDeduplicator, SyntheticCowDeduplicator};
use reclaim_core::mutation::PreconditionError;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_cow_deduplication_synthetic_happy_path() {
    let temp_dir = TempDir::new().unwrap();
    let keeper = temp_dir.path().join("keeper.txt");
    let duplicate = temp_dir.path().join("duplicate.txt");

    fs::write(&keeper, b"identical duplicate content block").unwrap();
    fs::write(&duplicate, b"identical duplicate content block").unwrap();

    let synthetic = SyntheticCowDeduplicator;
    assert!(synthetic.is_supported(&keeper));

    let result = synthetic.deduplicate(&keeper, &duplicate).unwrap();
    assert_eq!(result.source, keeper);
    assert_eq!(result.target, duplicate);
    assert_eq!(result.bytes_reclaimed, 33);
    assert!(!result.already_shared);
}

#[cfg(unix)]
#[test]
fn test_cow_deduplication_synthetic_already_shared_hard_link() {
    let temp_dir = TempDir::new().unwrap();
    let keeper = temp_dir.path().join("keeper.txt");
    let hard_link = temp_dir.path().join("hardlink.txt");

    fs::write(&keeper, b"shared inode data").unwrap();
    fs::hard_link(&keeper, &hard_link).unwrap();

    let synthetic = SyntheticCowDeduplicator;
    let result = synthetic.deduplicate(&keeper, &hard_link).unwrap();
    assert!(result.already_shared);
    assert_eq!(result.bytes_reclaimed, 0);
}

#[test]
fn test_cow_deduplication_precondition_hash_mismatch() {
    let temp_dir = TempDir::new().unwrap();
    let keeper = temp_dir.path().join("keeper.txt");
    let fake_duplicate = temp_dir.path().join("fake_dup.txt");

    fs::write(&keeper, b"keeper data 12345").unwrap();
    fs::write(&fake_duplicate, b"diff!! data 12345").unwrap();

    let synthetic = SyntheticCowDeduplicator;
    let err = synthetic.deduplicate(&keeper, &fake_duplicate).unwrap_err();
    match err {
        CowError::PreconditionFailed(PreconditionError::SourceHashMismatch { .. }) => {}
        other => panic!("Expected SourceHashMismatch, got: {:?}", other),
    }
}

#[test]
fn test_cow_deduplication_precondition_size_mismatch() {
    let temp_dir = TempDir::new().unwrap();
    let keeper = temp_dir.path().join("keeper.txt");
    let different_size = temp_dir.path().join("diff_size.txt");

    fs::write(&keeper, b"keeper content").unwrap();
    fs::write(&different_size, b"keeper content plus extra bytes").unwrap();

    let synthetic = SyntheticCowDeduplicator;
    let err = synthetic.deduplicate(&keeper, &different_size).unwrap_err();
    match err {
        CowError::PreconditionFailed(PreconditionError::SourceSizeMismatch { .. }) => {}
        other => panic!("Expected SourceSizeMismatch, got: {:?}", other),
    }
}

#[test]
fn test_cow_deduplication_missing_file() {
    let temp_dir = TempDir::new().unwrap();
    let keeper = temp_dir.path().join("keeper.txt");
    let missing_dup = temp_dir.path().join("nonexistent.txt");

    fs::write(&keeper, b"keeper content").unwrap();

    let synthetic = SyntheticCowDeduplicator;
    let err = synthetic.deduplicate(&keeper, &missing_dup).unwrap_err();
    match err {
        CowError::PreconditionFailed(PreconditionError::SourceMissing(_)) => {}
        other => panic!("Expected SourceMissing, got: {:?}", other),
    }
}

#[cfg(unix)]
#[test]
fn test_cow_deduplication_symlink_refusal() {
    let temp_dir = TempDir::new().unwrap();
    let keeper = temp_dir.path().join("keeper.txt");
    let real_file = temp_dir.path().join("real.txt");
    let symlink_dup = temp_dir.path().join("symlink_dup.txt");

    fs::write(&keeper, b"content").unwrap();
    fs::write(&real_file, b"content").unwrap();
    std::os::unix::fs::symlink(&real_file, &symlink_dup).unwrap();

    let synthetic = SyntheticCowDeduplicator;
    let err = synthetic.deduplicate(&keeper, &symlink_dup).unwrap_err();
    match err {
        CowError::PreconditionFailed(PreconditionError::SourceIsSymlink(_)) => {}
        other => panic!("Expected SourceIsSymlink, got: {:?}", other),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn test_cow_apfs_native_deduplication() {
    let temp_dir = TempDir::new().unwrap();
    let keeper = temp_dir.path().join("keeper.bin");
    let duplicate = temp_dir.path().join("duplicate.bin");

    let payload = vec![0x42u8; 1024 * 64]; // 64 KB
    fs::write(&keeper, &payload).unwrap();
    fs::write(&duplicate, &payload).unwrap();

    if ApfsCloneDeduplicator::is_apfs(temp_dir.path()) {
        let deduplicator = default_cow_deduplicator();
        assert!(deduplicator.is_supported(&keeper));

        let result = deduplicate_via_cow(&keeper, &duplicate).unwrap();
        assert_eq!(result.bytes_reclaimed, 1024 * 64);
        assert!(!result.already_shared);

        // Verify content integrity of both files post-deduplication
        assert_eq!(fs::read(&keeper).unwrap(), payload);
        assert_eq!(fs::read(&duplicate).unwrap(), payload);

        // Verify independent write behavior (Copy-on-Write)
        // Modifying duplicate should NOT mutate keeper!
        fs::write(&duplicate, b"modified duplicate payload").unwrap();
        assert_eq!(fs::read(&keeper).unwrap(), payload);
        assert_eq!(fs::read(&duplicate).unwrap(), b"modified duplicate payload");
    }
}
