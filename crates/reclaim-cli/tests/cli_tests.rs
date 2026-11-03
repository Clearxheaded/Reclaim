use std::fs;
use std::process::Command;
use tempfile::TempDir;

const RECLAIM_BIN: &str = env!("CARGO_BIN_EXE_reclaim");

#[test]
fn test_cli_undo_happy_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let canonical = root.join("canonical");
    let copies = root.join("copies");
    fs::create_dir_all(&canonical).unwrap();
    fs::create_dir_all(&copies).unwrap();

    let file_a = canonical.join("keeper.txt");
    let file_b = copies.join("duplicate_copy.txt");
    let content = "Exact duplicate payload for CLI undo test 12345";
    fs::write(&file_a, content).unwrap();
    fs::write(&file_b, content).unwrap();

    let journal_path = root.join(".reclaim").join("journal.db");
    let quarantine_path = root.join(".reclaim").join("quarantine");

    // 1. Execute quarantine
    let exec_output = Command::new(RECLAIM_BIN)
        .arg("execute")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .arg("--quarantine-dir")
        .arg(&quarantine_path)
        .output()
        .expect("failed to run reclaim execute");

    assert!(
        exec_output.status.success(),
        "execute failed: {}",
        String::from_utf8_lossy(&exec_output.stderr)
    );
    assert!(!file_b.exists(), "Duplicate file should have been moved");

    // 2. Run history command
    let history_output = Command::new(RECLAIM_BIN)
        .arg("history")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .output()
        .expect("failed to run reclaim history");

    assert!(history_output.status.success());
    let history_str = String::from_utf8_lossy(&history_output.stdout);
    assert!(history_str.contains("SESSION ID"));
    assert!(history_str.contains("Completed"));

    // 3. Run recover on clean state
    let recover_output = Command::new(RECLAIM_BIN)
        .arg("recover")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .output()
        .expect("failed to run reclaim recover");

    assert!(recover_output.status.success());
    let recover_str = String::from_utf8_lossy(&recover_output.stdout);
    assert!(recover_str.contains("clean"));

    // 4. Run undo dry run
    let dry_run_output = Command::new(RECLAIM_BIN)
        .arg("undo")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .arg("--last")
        .arg("--dry-run")
        .output()
        .expect("failed to run reclaim undo --dry-run");

    assert!(dry_run_output.status.success());
    let dry_str = String::from_utf8_lossy(&dry_run_output.stdout);
    assert!(dry_str.contains("DRY RUN VALIDATED"));
    assert!(!file_b.exists(), "Dry run must NOT restore file on disk");

    // 5. Run live undo
    let undo_output = Command::new(RECLAIM_BIN)
        .arg("undo")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .arg("--last")
        .output()
        .expect("failed to run reclaim undo");

    assert!(
        undo_output.status.success(),
        "undo failed: {}",
        String::from_utf8_lossy(&undo_output.stderr)
    );
    let undo_str = String::from_utf8_lossy(&undo_output.stdout);
    assert!(undo_str.contains("RESTORED"));

    // Verify file returned with byte-for-byte fidelity
    assert!(file_b.exists(), "Duplicate file should be restored");
    let restored = fs::read_to_string(&file_b).unwrap();
    assert_eq!(restored, content);
}

#[test]
fn test_cli_undo_conflict_occupied_destination() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let canonical = root.join("canonical");
    let copies = root.join("copies");
    fs::create_dir_all(&canonical).unwrap();
    fs::create_dir_all(&copies).unwrap();

    let file_a = canonical.join("keeper.txt");
    let file_b = copies.join("duplicate_copy.txt");
    let content = "Exact duplicate payload for CLI conflict test";
    fs::write(&file_a, content).unwrap();
    fs::write(&file_b, content).unwrap();

    let journal_path = root.join(".reclaim").join("journal.db");
    let quarantine_path = root.join(".reclaim").join("quarantine");

    // Execute quarantine
    let exec_output = Command::new(RECLAIM_BIN)
        .arg("execute")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .arg("--quarantine-dir")
        .arg(&quarantine_path)
        .output()
        .unwrap();
    assert!(exec_output.status.success());
    assert!(!file_b.exists());

    // Introduce foreign file at original destination
    let foreign = "FOREIGN CONTENT - DO NOT OVERWRITE";
    fs::write(&file_b, foreign).unwrap();

    // Run Undo
    let undo_output = Command::new(RECLAIM_BIN)
        .arg("undo")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .arg("--last")
        .output()
        .unwrap();

    // Must exit with error because of conflict
    assert!(!undo_output.status.success());
    let stderr_str = String::from_utf8_lossy(&undo_output.stderr);
    let stdout_str = String::from_utf8_lossy(&undo_output.stdout);
    let combined = format!("{}\n{}", stdout_str, stderr_str);
    assert!(combined.contains("CONFLICT"));
    assert!(combined.contains("already exists"));

    // Foreign file must be unchanged
    let current_content = fs::read_to_string(&file_b).unwrap();
    assert_eq!(current_content, foreign);
}

#[test]
fn test_cli_history_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let canonical = root.join("canonical");
    let copies = root.join("copies");
    fs::create_dir_all(&canonical).unwrap();
    fs::create_dir_all(&copies).unwrap();

    fs::write(canonical.join("a.txt"), "data").unwrap();
    fs::write(copies.join("b.txt"), "data").unwrap();

    let journal_path = root.join(".reclaim").join("journal.db");

    Command::new(RECLAIM_BIN)
        .arg("execute")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .output()
        .unwrap();

    let history_json = Command::new(RECLAIM_BIN)
        .arg("history")
        .arg(root)
        .arg("--journal")
        .arg(&journal_path)
        .arg("--json")
        .output()
        .unwrap();

    assert!(history_json.status.success());
    let stdout_str = String::from_utf8_lossy(&history_json.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout_str).unwrap();
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 1);
}
