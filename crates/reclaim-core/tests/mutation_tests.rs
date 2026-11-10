use reclaim_core::file_id::FileId;
use reclaim_core::mutation::{
    atomic_move_no_replace, compute_file_blake3_hash, validate_action_preconditions,
    verify_same_filesystem, MutationError, PreconditionError,
};
use reclaim_core::planner::{ActionPreconditions, QuarantineAction};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_atomic_move_success() {
    let temp_dir = TempDir::new().unwrap();
    let src = temp_dir.path().join("source.txt");
    let dst = temp_dir.path().join("destination.txt");

    fs::write(&src, b"important payload").unwrap();
    assert!(src.exists());
    assert!(!dst.exists());

    let result = atomic_move_no_replace(&src, &dst);
    assert!(result.is_ok(), "Move should succeed: {:?}", result.err());

    assert!(!src.exists(), "Source should no longer exist after move");
    assert!(dst.exists(), "Destination should exist after move");
    assert_eq!(fs::read(&dst).unwrap(), b"important payload");
}

#[test]
fn test_atomic_move_destination_collision() {
    let temp_dir = TempDir::new().unwrap();
    let src = temp_dir.path().join("source.txt");
    let dst = temp_dir.path().join("destination.txt");

    fs::write(&src, b"new source content").unwrap();
    fs::write(&dst, b"pre-existing destination content").unwrap();

    let result = atomic_move_no_replace(&src, &dst);
    assert!(
        result.is_err(),
        "Move should fail when destination already exists"
    );

    match result.unwrap_err() {
        MutationError::DestinationAlreadyExists(colliding_path) => {
            assert_eq!(colliding_path, dst);
        }
        other => panic!("Expected DestinationAlreadyExists error, got: {:?}", other),
    }

    // Critical invariant: Destination must NOT be overwritten!
    assert_eq!(
        fs::read(&dst).unwrap(),
        b"pre-existing destination content",
        "Destination must remain completely unmodified!"
    );
    // Critical invariant: Source must NOT be deleted or moved!
    assert!(
        src.exists(),
        "Source must remain intact when destination collision occurs"
    );
    assert_eq!(fs::read(&src).unwrap(), b"new source content");
}

#[test]
fn test_atomic_move_nonexistent_source() {
    let temp_dir = TempDir::new().unwrap();
    let src = temp_dir.path().join("does_not_exist.txt");
    let dst = temp_dir.path().join("dest.txt");

    let result = atomic_move_no_replace(&src, &dst);
    assert!(result.is_err());
    match result.unwrap_err() {
        MutationError::Io(err) => {
            assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("Expected Io(NotFound) error, got: {:?}", other),
    }
}

#[test]
fn test_verify_same_filesystem() {
    let temp_dir = TempDir::new().unwrap();
    let src = temp_dir.path().join("source.txt");
    let dst = temp_dir.path().join("quarantine/dest.txt");

    fs::write(&src, b"data").unwrap();
    fs::create_dir_all(dst.parent().unwrap()).unwrap();

    let result = verify_same_filesystem(&src, &dst);
    assert!(
        result.is_ok(),
        "Same temp directory must reside on the same filesystem"
    );
}

#[test]
fn test_validate_preconditions_happy_path() {
    let temp_dir = TempDir::new().unwrap();
    let keeper_path = temp_dir.path().join("keeper.txt");
    let source_path = temp_dir.path().join("duplicate.txt");
    let quarantine_path = temp_dir.path().join("quarantine/duplicate.txt");

    let content = b"identical duplicate content for test";
    fs::write(&keeper_path, content).unwrap();
    fs::write(&source_path, content).unwrap();

    let hash = compute_file_blake3_hash(&keeper_path).unwrap();
    let keeper_meta = fs::symlink_metadata(&keeper_path).unwrap();
    let source_meta = fs::symlink_metadata(&source_path).unwrap();

    let action = QuarantineAction {
        source_path: source_path.clone(),
        quarantine_path: quarantine_path.clone(),
        keeper_path: keeper_path.clone(),
        preconditions: ActionPreconditions {
            expected_size: content.len() as u64,
            expected_hash: hash,
            source_modified: source_meta.modified().ok(),
            source_file_id: FileId::from_path_and_metadata(&source_path, &source_meta),
            keeper_modified: keeper_meta.modified().ok(),
            keeper_file_id: FileId::from_path_and_metadata(&keeper_path, &keeper_meta),
        },
        rationale: "Quarantine duplicate".into(),
    };

    let result = validate_action_preconditions(&action);
    assert!(
        result.is_ok(),
        "Precondition validation should succeed: {:?}",
        result.err()
    );
}

#[test]
fn test_validate_preconditions_modified_source_hash() {
    let temp_dir = TempDir::new().unwrap();
    let keeper_path = temp_dir.path().join("keeper.txt");
    let source_path = temp_dir.path().join("duplicate.txt");
    let quarantine_path = temp_dir.path().join("quarantine/duplicate.txt");

    let content = b"original identical duplicate content";
    fs::write(&keeper_path, content).unwrap();
    fs::write(&source_path, content).unwrap();

    let hash = compute_file_blake3_hash(&keeper_path).unwrap();
    let keeper_meta = fs::symlink_metadata(&keeper_path).unwrap();
    let source_meta = fs::symlink_metadata(&source_path).unwrap();

    let action = QuarantineAction {
        source_path: source_path.clone(),
        quarantine_path,
        keeper_path: keeper_path.clone(),
        preconditions: ActionPreconditions {
            expected_size: content.len() as u64,
            expected_hash: hash,
            source_modified: source_meta.modified().ok(),
            source_file_id: FileId::from_path_and_metadata(&source_path, &source_meta),
            keeper_modified: keeper_meta.modified().ok(),
            keeper_file_id: FileId::from_path_and_metadata(&keeper_path, &keeper_meta),
        },
        rationale: "Quarantine duplicate".into(),
    };

    // Simulate TOCTOU: file was modified between planning and execution (same size, different content)
    fs::write(&source_path, b"tampered duplicate content altered!").unwrap();

    let result = validate_action_preconditions(&action);
    assert!(result.is_err());
    match result.unwrap_err() {
        PreconditionError::SourceHashMismatch { path, .. }
        | PreconditionError::SourceSizeMismatch { path, .. } => {
            assert_eq!(path, source_path);
        }
        other => panic!("Expected hash or size mismatch, got: {:?}", other),
    }
}

#[test]
fn test_validate_preconditions_missing_keeper() {
    let temp_dir = TempDir::new().unwrap();
    let keeper_path = temp_dir.path().join("keeper.txt");
    let source_path = temp_dir.path().join("duplicate.txt");
    let quarantine_path = temp_dir.path().join("quarantine/duplicate.txt");

    let content = b"duplicate test content";
    fs::write(&keeper_path, content).unwrap();
    fs::write(&source_path, content).unwrap();

    let hash = compute_file_blake3_hash(&keeper_path).unwrap();
    let keeper_meta = fs::symlink_metadata(&keeper_path).unwrap();
    let source_meta = fs::symlink_metadata(&source_path).unwrap();

    let action = QuarantineAction {
        source_path: source_path.clone(),
        quarantine_path,
        keeper_path: keeper_path.clone(),
        preconditions: ActionPreconditions {
            expected_size: content.len() as u64,
            expected_hash: hash,
            source_modified: source_meta.modified().ok(),
            source_file_id: FileId::from_path_and_metadata(&source_path, &source_meta),
            keeper_modified: keeper_meta.modified().ok(),
            keeper_file_id: FileId::from_path_and_metadata(&keeper_path, &keeper_meta),
        },
        rationale: "Quarantine duplicate".into(),
    };

    // Simulate keeper deletion
    fs::remove_file(&keeper_path).unwrap();

    let result = validate_action_preconditions(&action);
    assert!(result.is_err());
    match result.unwrap_err() {
        PreconditionError::KeeperMissing(path) => {
            assert_eq!(path, keeper_path);
        }
        other => panic!("Expected KeeperMissing error, got: {:?}", other),
    }
}

#[test]
fn test_validate_preconditions_missing_source() {
    let temp_dir = TempDir::new().unwrap();
    let keeper_path = temp_dir.path().join("keeper.txt");
    let source_path = temp_dir.path().join("duplicate.txt");
    let quarantine_path = temp_dir.path().join("quarantine/duplicate.txt");

    let content = b"duplicate test content";
    fs::write(&keeper_path, content).unwrap();
    fs::write(&source_path, content).unwrap();

    let hash = compute_file_blake3_hash(&keeper_path).unwrap();
    let keeper_meta = fs::symlink_metadata(&keeper_path).unwrap();
    let source_meta = fs::symlink_metadata(&source_path).unwrap();

    let action = QuarantineAction {
        source_path: source_path.clone(),
        quarantine_path,
        keeper_path: keeper_path.clone(),
        preconditions: ActionPreconditions {
            expected_size: content.len() as u64,
            expected_hash: hash,
            source_modified: source_meta.modified().ok(),
            source_file_id: FileId::from_path_and_metadata(&source_path, &source_meta),
            keeper_modified: keeper_meta.modified().ok(),
            keeper_file_id: FileId::from_path_and_metadata(&keeper_path, &keeper_meta),
        },
        rationale: "Quarantine duplicate".into(),
    };

    // Delete source
    fs::remove_file(&source_path).unwrap();

    let result = validate_action_preconditions(&action);
    assert!(result.is_err());
    match result.unwrap_err() {
        PreconditionError::SourceMissing(path) => {
            assert_eq!(path, source_path);
        }
        other => panic!("Expected SourceMissing error, got: {:?}", other),
    }
}

#[test]
fn test_validate_preconditions_destination_already_exists() {
    let temp_dir = TempDir::new().unwrap();
    let keeper_path = temp_dir.path().join("keeper.txt");
    let source_path = temp_dir.path().join("duplicate.txt");
    let quarantine_path = temp_dir.path().join("quarantine/duplicate.txt");

    let content = b"duplicate test content";
    fs::write(&keeper_path, content).unwrap();
    fs::write(&source_path, content).unwrap();

    let hash = compute_file_blake3_hash(&keeper_path).unwrap();
    let keeper_meta = fs::symlink_metadata(&keeper_path).unwrap();
    let source_meta = fs::symlink_metadata(&source_path).unwrap();

    let action = QuarantineAction {
        source_path: source_path.clone(),
        quarantine_path: quarantine_path.clone(),
        keeper_path: keeper_path.clone(),
        preconditions: ActionPreconditions {
            expected_size: content.len() as u64,
            expected_hash: hash,
            source_modified: source_meta.modified().ok(),
            source_file_id: FileId::from_path_and_metadata(&source_path, &source_meta),
            keeper_modified: keeper_meta.modified().ok(),
            keeper_file_id: FileId::from_path_and_metadata(&keeper_path, &keeper_meta),
        },
        rationale: "Quarantine duplicate".into(),
    };

    // Pre-create file in quarantine path
    fs::create_dir_all(quarantine_path.parent().unwrap()).unwrap();
    fs::write(&quarantine_path, b"already existing quarantine file").unwrap();

    let result = validate_action_preconditions(&action);
    assert!(result.is_err());
    match result.unwrap_err() {
        PreconditionError::DestinationAlreadyExists(path) => {
            assert_eq!(path, quarantine_path);
        }
        other => panic!("Expected DestinationAlreadyExists error, got: {:?}", other),
    }
}

#[cfg(unix)]
#[test]
fn test_validate_preconditions_symlink_defense() {
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new().unwrap();
    let real_file = temp_dir.path().join("real.txt");
    let keeper_path = temp_dir.path().join("keeper.txt");
    let source_path = temp_dir.path().join("duplicate_symlink.txt");
    let quarantine_path = temp_dir.path().join("quarantine/duplicate.txt");

    let content = b"duplicate content";
    fs::write(&real_file, content).unwrap();
    fs::write(&keeper_path, content).unwrap();

    let hash = compute_file_blake3_hash(&keeper_path).unwrap();
    let keeper_meta = fs::symlink_metadata(&keeper_path).unwrap();

    // Create a symlink pointing to real_file
    symlink(&real_file, &source_path).unwrap();

    let action = QuarantineAction {
        source_path: source_path.clone(),
        quarantine_path,
        keeper_path: keeper_path.clone(),
        preconditions: ActionPreconditions {
            expected_size: content.len() as u64,
            expected_hash: hash,
            source_modified: None,
            source_file_id: None,
            keeper_modified: keeper_meta.modified().ok(),
            keeper_file_id: FileId::from_path_and_metadata(&keeper_path, &keeper_meta),
        },
        rationale: "Quarantine duplicate".into(),
    };

    let result = validate_action_preconditions(&action);
    assert!(result.is_err());
    match result.unwrap_err() {
        PreconditionError::SourceIsSymlink(path) => {
            assert_eq!(path, source_path);
        }
        other => panic!("Expected SourceIsSymlink error, got: {:?}", other),
    }
}
