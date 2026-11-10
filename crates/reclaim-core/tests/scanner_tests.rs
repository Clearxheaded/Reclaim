use reclaim_core::{scan_directory, CancellationToken, RootScanError, ScanOptions};
#[cfg(unix)]
use reclaim_core::{FileId, ScanErrorKind, SkipReason};
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
        let path = std::env::temp_dir().join(format!("reclaim_test_{}_{}", name, unique));
        fs::create_dir_all(&path).expect("failed to create test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn create_sub_dir(&self, relative_path: impl AsRef<Path>) -> PathBuf {
        let p = self.path.join(relative_path);
        fs::create_dir_all(&p).expect("failed to create sub directory");
        p
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Restore permissions on subdirs in case a test removed them
            if let Ok(entries) = fs::read_dir(&self.path) {
                for entry in entries.flatten() {
                    let _ = fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o755));
                }
            }
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_empty_directory() {
    let fixture = TestDir::new("empty");
    let options = ScanOptions::default();

    let output = scan_directory(fixture.path(), options).expect("scan should succeed");

    assert_eq!(output.summary.directories_visited, 1);
    assert_eq!(output.summary.files_discovered, 0);
    assert_eq!(output.summary.total_file_bytes, 0);
    assert_eq!(output.summary.symlinks_skipped, 0);
    assert_eq!(output.summary.other_skipped, 0);
    assert_eq!(output.summary.errors_count, 0);
    assert!(!output.summary.was_cancelled);
    assert!(output.entries.is_empty());
    assert!(output.errors.is_empty());
    assert!(output.skipped.is_empty());
}

#[test]
fn test_nested_directories_and_byte_counts() {
    let fixture = TestDir::new("nested");
    fixture.create_file("root_file.txt", b"12345"); // 5 bytes
    fixture.create_file("dir1/file_a.txt", b"hello"); // 5 bytes
    fixture.create_file("dir1/dir2/file_b.txt", b"world!!"); // 7 bytes
    fixture.create_file("dir1/dir2/empty.txt", b""); // 0 bytes
    fixture.create_sub_dir("dir1/dir2/empty_sub"); // empty dir

    let output =
        scan_directory(fixture.path(), ScanOptions::default()).expect("scan should succeed");

    assert_eq!(output.summary.directories_visited, 4); // root, dir1, dir2, empty_sub
    assert_eq!(output.summary.files_discovered, 4);
    assert_eq!(output.summary.total_file_bytes, 17); // 5 + 5 + 7 + 0 = 17 bytes
    assert_eq!(output.entries.len(), 4);
    assert_eq!(output.summary.errors_count, 0);
}

#[test]
fn test_unicode_filenames() {
    let fixture = TestDir::new("unicode");
    fixture.create_file("🦀_rust_ferris.txt", b"crab");
    fixture.create_file("naïve_café_résumé.txt", b"coffee");
    fixture.create_file("日本語_テスト.txt", b"nihongo");
    fixture.create_file("русский_текст.txt", b"russian");
    fixture.create_file("مرحبا.txt", b"arabic");

    let output =
        scan_directory(fixture.path(), ScanOptions::default()).expect("scan should succeed");

    assert_eq!(output.summary.files_discovered, 5);
    assert_eq!(output.entries.len(), 5);

    let filenames: Vec<String> = output
        .entries
        .iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();

    assert!(filenames.iter().any(|f| f.contains("🦀")));
    assert!(filenames.iter().any(|f| f.contains("naïve")));
    assert!(filenames.iter().any(|f| f.contains("日本語")));
    assert!(filenames.iter().any(|f| f.contains("русский")));
    assert!(filenames.iter().any(|f| f.contains("مرحبا")));
}

#[cfg(unix)]
#[test]
fn test_symbolic_links_are_skipped() {
    use std::os::unix::fs::symlink;

    let fixture = TestDir::new("symlinks");
    let real_file = fixture.create_file("real.txt", b"real content");
    let real_dir = fixture.create_sub_dir("real_dir");
    fixture.create_file("real_dir/nested.txt", b"inside real dir");

    let symlink_file = fixture.path().join("symlink_to_file.txt");
    symlink(&real_file, &symlink_file).expect("failed to create file symlink");

    let symlink_dir = fixture.path().join("symlink_to_dir");
    symlink(&real_dir, &symlink_dir).expect("failed to create dir symlink");

    let output =
        scan_directory(fixture.path(), ScanOptions::default()).expect("scan should succeed");

    // The real dir was visited; the symlink dir must NEVER be traversed.
    assert_eq!(output.summary.symlinks_skipped, 2);
    assert_eq!(output.skipped.len(), 2);
    assert_eq!(output.summary.files_discovered, 2); // real.txt and real_dir/nested.txt only!

    for skipped in &output.skipped {
        assert_eq!(skipped.reason, SkipReason::SymbolicLink);
    }
}

#[cfg(unix)]
#[test]
fn test_root_is_symlink_refused() {
    use std::os::unix::fs::symlink;

    let fixture = TestDir::new("root_symlink");
    let real_dir = fixture.create_sub_dir("actual_dir");
    let root_link = fixture.path().join("link_to_root");
    symlink(&real_dir, &root_link).expect("failed to create symlink");

    let result = scan_directory(&root_link, ScanOptions::default());
    match result {
        Err(RootScanError::IsSymbolicLink(p)) => {
            assert_eq!(p, root_link);
        }
        other => panic!("expected RootScanError::IsSymbolicLink, got {:?}", other),
    }
}

#[test]
fn test_root_not_found() {
    let non_existent = std::env::temp_dir().join("reclaim_non_existent_path_987654321");
    let result = scan_directory(&non_existent, ScanOptions::default());
    match result {
        Err(RootScanError::NotFound(p)) => {
            assert_eq!(p, non_existent);
        }
        other => panic!("expected RootScanError::NotFound, got {:?}", other),
    }
}

#[test]
fn test_root_is_file_refused() {
    let fixture = TestDir::new("root_file");
    let file = fixture.create_file("plain.txt", b"not a dir");

    let result = scan_directory(&file, ScanOptions::default());
    match result {
        Err(RootScanError::NotADirectory(p)) => {
            assert_eq!(p, file);
        }
        other => panic!("expected RootScanError::NotADirectory, got {:?}", other),
    }
}

#[test]
fn test_cancellation() {
    let fixture = TestDir::new("cancellation");
    for i in 0..20 {
        fixture.create_file(format!("dir_{}/file.txt", i), b"content");
    }

    let token = CancellationToken::new();
    let token_clone = token.clone();
    let visited_counter = Arc::new(AtomicUsize::new(0));
    let visited_counter_clone = visited_counter.clone();

    let options = ScanOptions {
        cancellation_token: Some(token),
        progress_callback: Some(Box::new(move |_progress| {
            let count = visited_counter_clone.fetch_add(1, Ordering::Relaxed);
            if count >= 3 {
                token_clone.cancel();
            }
        })),
    };

    let output = scan_directory(fixture.path(), options).expect("scan should return output");

    assert!(output.summary.was_cancelled);
    assert!(output.summary.directories_visited < 21);
}

#[cfg(unix)]
#[test]
fn test_permission_denied_directory_handled_gracefully() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = TestDir::new("permission_denied");
    fixture.create_file("readable/ok.txt", b"all good");

    let locked_dir = fixture.create_sub_dir("locked");
    fixture.create_file("locked/secret.txt", b"top secret");

    // Remove read and execute permissions from locked directory
    fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o000))
        .expect("failed to set permissions");

    let output =
        scan_directory(fixture.path(), ScanOptions::default()).expect("scan should not crash");

    // Restore permissions so cleanup works
    let _ = fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o755));

    // The scan must not crash; it should report the error and still find the readable file
    assert_eq!(output.summary.files_discovered, 1);
    assert_eq!(output.summary.errors_count, 1);
    assert_eq!(output.errors.len(), 1);
    assert_eq!(output.errors[0].kind, ScanErrorKind::PermissionDenied);
}

#[cfg(unix)]
#[test]
fn test_hard_link_identity_matches() {
    let fixture = TestDir::new("hardlink");
    let original = fixture.create_file("original.txt", b"same content");
    let hard_link = fixture.path().join("hardlink.txt");
    fs::hard_link(&original, &hard_link).expect("failed to create hard link");

    let output =
        scan_directory(fixture.path(), ScanOptions::default()).expect("scan should succeed");

    assert_eq!(output.entries.len(), 2);

    let original_entry = output
        .entries
        .iter()
        .find(|e| e.path == original)
        .expect("original entry found");
    let hardlink_entry = output
        .entries
        .iter()
        .find(|e| e.path == hard_link)
        .expect("hardlink entry found");

    assert_eq!(original_entry.file_id, hardlink_entry.file_id);
    #[cfg(unix)]
    assert!(matches!(original_entry.file_id, Some(FileId::Unix { .. })));
    #[cfg(windows)]
    assert!(matches!(original_entry.file_id, Some(FileId::Windows { .. })));
}
