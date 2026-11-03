use reclaim_core::{
    find_duplicates, generate_plan, scan_directory, CleanupPlan, DuplicateOptions,
    DuplicateSummary, KeeperPolicy, PlanOptions, PlanSummary, RootScanError, ScanOptions,
    ScanSummary,
};
use reclaim_storage::{
    ActionJournal, ExecutionOptions, ExecutionProgressEvent, ExecutionReport, SafeExecutor,
    SafeReverter, SessionId, UndoOptions, UndoProgressEvent, UndoReport,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        return ExitCode::FAILURE;
    }

    match args[1].as_str() {
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        "-v" | "--version" | "version" => {
            println!("reclaim {}", VERSION);
            ExitCode::SUCCESS
        }
        "scan" => {
            if args.len() < 3 {
                eprintln!("Error: 'scan' requires a directory path.");
                eprintln!("Usage: reclaim scan <directory>");
                return ExitCode::FAILURE;
            }
            let target_str = &args[2];
            let target_path = PathBuf::from(target_str);
            run_scan(&target_path)
        }
        "duplicates" => {
            if args.len() < 3 {
                eprintln!("Error: 'duplicates' requires a directory path.");
                eprintln!("Usage: reclaim duplicates <directory> [--include-empty]");
                return ExitCode::FAILURE;
            }

            let mut target_str = None;
            let mut include_empty = false;

            for arg in &args[2..] {
                match arg.as_str() {
                    "--include-empty" => include_empty = true,
                    "-h" | "--help" => {
                        println!("Usage: reclaim duplicates <directory> [--include-empty]");
                        println!();
                        println!(
                            "Finds exact duplicate files using two-stage BLAKE3 content hashing."
                        );
                        println!("Accounts for hard links to avoid inflated storage savings.");
                        println!();
                        println!("Options:");
                        println!(
                            "  --include-empty    Include 0-byte files in duplicate detection"
                        );
                        return ExitCode::SUCCESS;
                    }
                    path if target_str.is_none() => target_str = Some(path.to_string()),
                    other => {
                        eprintln!("Error: unrecognized argument '{}'", other);
                        eprintln!("Usage: reclaim duplicates <directory> [--include-empty]");
                        return ExitCode::FAILURE;
                    }
                }
            }

            match target_str {
                Some(dir) => run_duplicates(&PathBuf::from(dir), include_empty),
                None => {
                    eprintln!("Error: 'duplicates' requires a directory path.");
                    eprintln!("Usage: reclaim duplicates <directory> [--include-empty]");
                    ExitCode::FAILURE
                }
            }
        }
        "plan" => parse_and_run_plan(&args[2..]),
        "execute" => parse_and_run_execute(&args[2..]),
        "undo" => parse_and_run_undo(&args[2..]),
        "recover" => parse_and_run_recover(&args[2..]),
        "history" => parse_and_run_history(&args[2..]),
        unknown => {
            eprintln!("Error: unrecognized command '{}'", unknown);
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    println!(
        "Reclaim — Safe, reviewable filesystem cleanup (v{})",
        VERSION
    );
    println!();
    println!("USAGE:");
    println!("    reclaim <COMMAND> [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!(
        "    scan <directory>                  Recursively scan a directory in read-only mode"
    );
    println!("    duplicates <directory> [OPTIONS]  Identify exact duplicate files using BLAKE3");
    println!("    plan <directory> [OPTIONS]        Generate an immutable cleanup plan preview");
    println!("    execute <plan.json|dir> [OPTIONS] Execute safe quarantine moves with atomic non-replace");
    println!(
        "    undo [directory] [OPTIONS]        Safely restore quarantined files with TOCTOU checks"
    );
    println!("    recover [directory] [OPTIONS]     Reconcile interrupted actions against physical disk state");
    println!("    history [directory] [OPTIONS]     Inspect previous cleanup sessions and journal audit trails");
    println!("    help                              Print this help information");
    println!("    version                           Print version");
    println!();
    println!("EXECUTE OPTIONS:");
    println!("    --dry-run                         Simulate execution without modifying files");
    println!("    --journal <path>                  SQLite action journal database path");
    println!("    --json                            Output execution report as structured JSON");
    println!("    --halt-on-error                   Halt immediately on first action failure");
    println!();
    println!("UNDO OPTIONS:");
    println!("    --session <uuid>                  Undo a specific session by UUID");
    println!("    --last                            Undo the most recent completed execution session [default]");
    println!("    --dry-run                         Simulate restoration and check conflicts without moving files");
    println!("    --journal <path>                  SQLite action journal database path");
    println!("    --json                            Output undo report as structured JSON");
    println!("    --halt-on-error                   Halt immediately on first conflict or error");
    println!();
    println!("PLAN OPTIONS:");
    println!(
        "    --policy <policy>                 Keeper selection policy (shortest-path, oldest, newest) [default: shortest-path]"
    );
    println!("    --prefer-dir <dir>                Preferred keeper canonical directory");
    println!(
        "    --quarantine-dir <dir>            Target quarantine directory [default: <root>/.reclaim/quarantine]"
    );
    println!("    --json                            Output cleanup plan as structured JSON");
    println!("    --include-empty                   Include 0-byte files in duplicate evaluation");
    println!();
    println!("GLOBAL OPTIONS:");
    println!("    -h, --help                        Print help");
    println!("    -v, --version                     Print version");
}

fn print_plan_help() {
    println!("Usage: reclaim plan <directory> [OPTIONS]");
    println!();
    println!("Generates an immutable, prospective cleanup plan without modifying any files.");
    println!(
        "Selects keeper files, captures execution preconditions, and reports reclaimable space."
    );
    println!();
    println!("Options:");
    println!("    --policy <policy>       Keeper selection policy [default: shortest-path]");
    println!("                            Values: shortest-path, oldest, newest");
    println!("    --prefer-dir <dir>      Protect files residing in this canonical directory");
    println!(
        "    --quarantine-dir <dir>  Staged quarantine folder [default: <root>/.reclaim/quarantine]"
    );
    println!("    --json                  Emit plan preview as formatted JSON");
    println!("    --include-empty         Include 0-byte files in duplicate detection");
    println!("    -h, --help              Print this help information");
}

fn print_execute_help() {
    println!("Usage: reclaim execute <plan.json | directory> [OPTIONS]");
    println!();
    println!("Executes safe, reviewable quarantine moves with atomic no-replace guarantees");
    println!("and durable SQLite state machine journaling.");
    println!();
    println!("Options:");
    println!("    --dry-run               Simulate pre-validation and journal logging without moving files");
    println!("    --journal <path>        Custom SQLite journal database path [default: <root>/.reclaim/journal.db]");
    println!("    --json                  Emit machine-readable execution report as JSON");
    println!("    --halt-on-error         Halt execution on first action failure");
    println!("    --policy <policy>       Keeper selection policy if targeting a directory [default: shortest-path]");
    println!("    --prefer-dir <dir>      Preferred keeper directory if targeting a directory");
    println!("    --quarantine-dir <dir>  Target quarantine staging directory");
    println!("    --include-empty         Include 0-byte files if generating plan on the fly");
    println!("    -h, --help              Print this help information");
}

fn print_undo_help() {
    println!("Usage: reclaim undo [directory] [OPTIONS]");
    println!();
    println!("Safely restores quarantined duplicate files back to their original locations.");
    println!(
        "Enforces strict TOCTOU vacancy checks, keeper verification, and content hash validation."
    );
    println!();
    println!("Options:");
    println!("    --session <uuid>    Undo a specific session by its UUID");
    println!("    --last              Undo the most recent completed execution session [default]");
    println!(
        "    --dry-run           Simulate restoration and check conflicts without moving files"
    );
    println!("    --journal <path>    Custom SQLite journal database path [default: <root>/.reclaim/journal.db]");
    println!("    --json              Emit machine-readable undo report as formatted JSON");
    println!("    --halt-on-error     Halt execution on the first conflict or error");
    println!("    -h, --help          Print this help information");
}

fn print_recover_help() {
    println!("Usage: reclaim recover [directory] [OPTIONS]");
    println!();
    println!("Performs active dual journal-filesystem reconciliation after an unclean shutdown or crash.");
    println!("Inspects physical disk state at both source and quarantine paths to reconcile in-flight actions.");
    println!();
    println!("Options:");
    println!("    --journal <path>    Custom SQLite journal database path [default: <root>/.reclaim/journal.db]");
    println!("    --json              Emit machine-readable recovery report as formatted JSON");
    println!("    -h, --help          Print this help information");
}

fn print_history_help() {
    println!("Usage: reclaim history [directory] [OPTIONS]");
    println!();
    println!(
        "Displays past execution sessions, plans, action states, and reclaimed storage metrics."
    );
    println!();
    println!("Options:");
    println!("    --session <uuid>    Inspect detailed plans and actions for a specific session");
    println!("    --journal <path>    Custom SQLite journal database path [default: <root>/.reclaim/journal.db]");
    println!("    --json              Emit machine-readable session history as formatted JSON");
    println!("    -h, --help          Print this help information");
}

fn print_root_error_notes(err: &RootScanError) {
    match err {
        RootScanError::IsSymbolicLink(_) => {
            eprintln!(
                "Note: Reclaim refuses to scan root symlinks to prevent unexpected traversal."
            );
        }
        RootScanError::IsReparsePoint(_) => {
            eprintln!("Note: Reclaim refuses to scan root reparse points / junctions for safety.");
        }
        _ => {}
    }
}

fn run_scan(target_path: &Path) -> ExitCode {
    println!("Reclaim Scanner v{}", VERSION);
    println!("Target: {}", target_path.display());
    println!("Starting read-only filesystem scan...");
    println!();

    let options = ScanOptions::default();

    match scan_directory(target_path, options) {
        Ok(output) => {
            print_summary(&output.summary);

            if !output.skipped.is_empty() {
                println!();
                println!("Skipped Entries ({} total):", output.skipped.len());
                for skipped in output.skipped.iter().take(20) {
                    println!("  - [{}] {}", skipped.reason, skipped.path.display());
                }
                if output.skipped.len() > 20 {
                    println!("  ... and {} more", output.skipped.len() - 20);
                }
            }

            if !output.errors.is_empty() {
                println!();
                eprintln!(
                    "Non-fatal errors encountered during scan ({} total):",
                    output.errors.len()
                );
                for err in output.errors.iter().take(20) {
                    eprintln!("  - {}", err);
                }
                if output.errors.len() > 20 {
                    eprintln!("  ... and {} more", output.errors.len() - 20);
                }
            }

            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Error: {}", err);
            print_root_error_notes(&err);
            ExitCode::FAILURE
        }
    }
}

fn run_duplicates(target_path: &Path, include_empty: bool) -> ExitCode {
    println!("Reclaim Duplicate Detector v{}", VERSION);
    println!("Target: {}", target_path.display());
    println!("Scanning filesystem metadata in read-only mode...");

    let scan_opts = ScanOptions::default();
    let scan_output = match scan_directory(target_path, scan_opts) {
        Ok(output) => output,
        Err(err) => {
            eprintln!("Error scanning directory: {}", err);
            print_root_error_notes(&err);
            return ExitCode::FAILURE;
        }
    };

    println!(
        "Discovered {} regular files ({})",
        scan_output.summary.files_discovered,
        format_bytes(scan_output.summary.total_file_bytes)
    );
    println!("Evaluating candidates via two-stage BLAKE3 content hashing...");
    println!();

    let dup_opts = DuplicateOptions {
        include_empty,
        ..Default::default()
    };

    let dup_output = find_duplicates(&scan_output.entries, &dup_opts);

    if dup_output.groups.is_empty() {
        println!("No duplicate files found.");
        println!();
    } else {
        println!(
            "Found {} exact duplicate group(s):",
            dup_output.groups.len()
        );
        println!();

        for (idx, group) in dup_output.groups.iter().enumerate() {
            println!(
                "Group #{} — BLAKE3: {} | File Size: {} ({} bytes)",
                idx + 1,
                group.hash,
                format_bytes(group.file_size),
                group.file_size
            );
            println!(
                "  Distinct Physical Copies: {} | Paths: {} | Reclaimable: {}",
                group.distinct_physical_copies,
                group.file_count(),
                format_bytes(group.reclaimable_bytes)
            );

            for (copy_idx, copy) in group.physical_copies.iter().enumerate() {
                let id_str = match &copy.file_id {
                    Some(id) => format!(" [{}]", id),
                    None => String::new(),
                };

                println!("  Copy #{}{}:", copy_idx + 1, id_str);
                for path in &copy.paths {
                    println!("    - {}", path.display());
                }
            }
            println!();
        }
    }

    if !dup_output.errors.is_empty() {
        eprintln!(
            "Warnings/Errors encountered during hashing ({} total):",
            dup_output.errors.len()
        );
        for err in &dup_output.errors {
            eprintln!("  - {}", err);
        }
        println!();
    }

    print_duplicate_summary(&dup_output.summary);
    ExitCode::SUCCESS
}

fn parse_and_run_plan(raw_args: &[String]) -> ExitCode {
    if raw_args.is_empty() {
        eprintln!("Error: 'plan' requires a directory path.");
        eprintln!("Usage: reclaim plan <directory> [OPTIONS]");
        return ExitCode::FAILURE;
    }

    let mut target_str = None;
    let mut policy_str = "shortest-path".to_string();
    let mut prefer_dir_str = None;
    let mut quarantine_dir_str = None;
    let mut json_output = false;
    let mut include_empty = false;

    let mut i = 0;
    while i < raw_args.len() {
        let arg = &raw_args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_plan_help();
                return ExitCode::SUCCESS;
            }
            "--json" => {
                json_output = true;
                i += 1;
            }
            "--include-empty" => {
                include_empty = true;
                i += 1;
            }
            "--policy" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --policy requires a value (shortest-path, oldest, newest)");
                    return ExitCode::FAILURE;
                }
                policy_str = raw_args[i].clone();
                i += 1;
            }
            val if val.starts_with("--policy=") => {
                policy_str = val.trim_start_matches("--policy=").to_string();
                i += 1;
            }
            "--prefer-dir" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --prefer-dir requires a directory path");
                    return ExitCode::FAILURE;
                }
                prefer_dir_str = Some(raw_args[i].clone());
                i += 1;
            }
            val if val.starts_with("--prefer-dir=") => {
                prefer_dir_str = Some(val.trim_start_matches("--prefer-dir=").to_string());
                i += 1;
            }
            "--quarantine-dir" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --quarantine-dir requires a directory path");
                    return ExitCode::FAILURE;
                }
                quarantine_dir_str = Some(raw_args[i].clone());
                i += 1;
            }
            val if val.starts_with("--quarantine-dir=") => {
                quarantine_dir_str = Some(val.trim_start_matches("--quarantine-dir=").to_string());
                i += 1;
            }
            other if other.starts_with('-') => {
                eprintln!("Error: unrecognized option '{}'", other);
                return ExitCode::FAILURE;
            }
            path if target_str.is_none() => {
                target_str = Some(path.to_string());
                i += 1;
            }
            other => {
                eprintln!("Error: unexpected argument '{}'", other);
                return ExitCode::FAILURE;
            }
        }
    }

    let target_dir = match target_str {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("Error: 'plan' requires a directory path.");
            eprintln!("Usage: reclaim plan <directory> [OPTIONS]");
            return ExitCode::FAILURE;
        }
    };

    let base_policy = match policy_str.to_lowercase().as_str() {
        "shortest-path" | "shortest" | "shortestpath" => KeeperPolicy::ShortestPath,
        "oldest" | "oldest-modified" | "oldestmodified" => KeeperPolicy::OldestModified,
        "newest" | "newest-modified" | "newestmodified" => KeeperPolicy::NewestModified,
        other => {
            eprintln!(
                "Error: unrecognized keeper policy '{}'. Allowed values: shortest-path, oldest, newest",
                other
            );
            return ExitCode::FAILURE;
        }
    };

    let policy = match prefer_dir_str {
        Some(preferred_path) => {
            let pref_dir = PathBuf::from(preferred_path);
            if base_policy == KeeperPolicy::ShortestPath {
                KeeperPolicy::PreferredDirectory(pref_dir)
            } else {
                KeeperPolicy::PreferredDirectoryWithStrategy {
                    preferred_dir: pref_dir,
                    strategy: Box::new(base_policy),
                }
            }
        }
        None => base_policy,
    };

    let quarantine_dir = quarantine_dir_str.map(PathBuf::from);

    run_plan(
        &target_dir,
        policy,
        quarantine_dir,
        include_empty,
        json_output,
    )
}

fn run_plan(
    target_path: &Path,
    policy: KeeperPolicy,
    quarantine_dir: Option<PathBuf>,
    include_empty: bool,
    json_output: bool,
) -> ExitCode {
    if !json_output {
        println!("Reclaim Cleanup Planner v{}", VERSION);
        println!("Target root: {}", target_path.display());
        println!("Keeper policy: {}", policy.name());
        println!("Scanning filesystem metadata in read-only mode...");
    }

    let scan_opts = ScanOptions::default();
    let scan_output = match scan_directory(target_path, scan_opts) {
        Ok(output) => output,
        Err(err) => {
            eprintln!("Error scanning directory: {}", err);
            print_root_error_notes(&err);
            return ExitCode::FAILURE;
        }
    };

    let dup_opts = DuplicateOptions {
        include_empty,
        ..Default::default()
    };
    let dup_output = find_duplicates(&scan_output.entries, &dup_opts);

    let plan_opts = PlanOptions {
        root_dir: target_path.to_path_buf(),
        policy,
        quarantine_dir,
    };

    let plan = match generate_plan(&dup_output.groups, &plan_opts) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("Error generating cleanup plan: {}", err);
            return ExitCode::FAILURE;
        }
    };

    if json_output {
        match plan.to_json_pretty() {
            Ok(json_str) => {
                println!("{}", json_str);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("Error serializing plan to JSON: {}", err);
                ExitCode::FAILURE
            }
        }
    } else {
        print_plan_human(&plan);
        ExitCode::SUCCESS
    }
}

fn parse_and_run_execute(raw_args: &[String]) -> ExitCode {
    if raw_args.is_empty() {
        eprintln!("Error: 'execute' requires a plan JSON file or target directory path.");
        eprintln!("Usage: reclaim execute <plan.json | directory> [OPTIONS]");
        return ExitCode::FAILURE;
    }

    let mut target_str = None;
    let mut dry_run = false;
    let mut journal_str = None;
    let mut json_output = false;
    let mut halt_on_error = false;
    let mut policy_str = "shortest-path".to_string();
    let mut prefer_dir_str = None;
    let mut quarantine_dir_str = None;
    let mut include_empty = false;

    let mut i = 0;
    while i < raw_args.len() {
        let arg = &raw_args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_execute_help();
                return ExitCode::SUCCESS;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--json" => {
                json_output = true;
                i += 1;
            }
            "--halt-on-error" => {
                halt_on_error = true;
                i += 1;
            }
            "--include-empty" => {
                include_empty = true;
                i += 1;
            }
            "--journal" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --journal requires a path");
                    return ExitCode::FAILURE;
                }
                journal_str = Some(raw_args[i].clone());
                i += 1;
            }
            val if val.starts_with("--journal=") => {
                journal_str = Some(val.trim_start_matches("--journal=").to_string());
                i += 1;
            }
            "--policy" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --policy requires a value (shortest-path, oldest, newest)");
                    return ExitCode::FAILURE;
                }
                policy_str = raw_args[i].clone();
                i += 1;
            }
            val if val.starts_with("--policy=") => {
                policy_str = val.trim_start_matches("--policy=").to_string();
                i += 1;
            }
            "--prefer-dir" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --prefer-dir requires a directory path");
                    return ExitCode::FAILURE;
                }
                prefer_dir_str = Some(raw_args[i].clone());
                i += 1;
            }
            val if val.starts_with("--prefer-dir=") => {
                prefer_dir_str = Some(val.trim_start_matches("--prefer-dir=").to_string());
                i += 1;
            }
            "--quarantine-dir" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --quarantine-dir requires a directory path");
                    return ExitCode::FAILURE;
                }
                quarantine_dir_str = Some(raw_args[i].clone());
                i += 1;
            }
            val if val.starts_with("--quarantine-dir=") => {
                quarantine_dir_str = Some(val.trim_start_matches("--quarantine-dir=").to_string());
                i += 1;
            }
            other if other.starts_with('-') => {
                eprintln!("Error: unrecognized option '{}'", other);
                return ExitCode::FAILURE;
            }
            path if target_str.is_none() => {
                target_str = Some(path.to_string());
                i += 1;
            }
            other => {
                eprintln!("Error: unexpected argument '{}'", other);
                return ExitCode::FAILURE;
            }
        }
    }

    let target = match target_str {
        Some(t) => PathBuf::from(t),
        None => {
            eprintln!("Error: 'execute' requires a plan JSON file or target directory path.");
            eprintln!("Usage: reclaim execute <plan.json | directory> [OPTIONS]");
            return ExitCode::FAILURE;
        }
    };

    let base_policy = match policy_str.to_lowercase().as_str() {
        "shortest-path" | "shortest" | "shortestpath" => KeeperPolicy::ShortestPath,
        "oldest" | "oldest-modified" | "oldestmodified" => KeeperPolicy::OldestModified,
        "newest" | "newest-modified" | "newestmodified" => KeeperPolicy::NewestModified,
        other => {
            eprintln!(
                "Error: unrecognized keeper policy '{}'. Allowed values: shortest-path, oldest, newest",
                other
            );
            return ExitCode::FAILURE;
        }
    };

    let policy = match prefer_dir_str {
        Some(preferred_path) => {
            let pref_dir = PathBuf::from(preferred_path);
            if base_policy == KeeperPolicy::ShortestPath {
                KeeperPolicy::PreferredDirectory(pref_dir)
            } else {
                KeeperPolicy::PreferredDirectoryWithStrategy {
                    preferred_dir: pref_dir,
                    strategy: Box::new(base_policy),
                }
            }
        }
        None => base_policy,
    };

    let quarantine_dir = quarantine_dir_str.map(PathBuf::from);
    let custom_journal = journal_str.map(PathBuf::from);

    let cli_args = ExecuteCliArgs {
        target_path: target,
        policy,
        quarantine_dir,
        custom_journal,
        dry_run,
        halt_on_error,
        include_empty,
        json_output,
    };

    run_execute(cli_args)
}

struct ExecuteCliArgs {
    target_path: PathBuf,
    policy: KeeperPolicy,
    quarantine_dir: Option<PathBuf>,
    custom_journal: Option<PathBuf>,
    dry_run: bool,
    halt_on_error: bool,
    include_empty: bool,
    json_output: bool,
}

fn run_execute(args: ExecuteCliArgs) -> ExitCode {
    if !args.target_path.exists() {
        eprintln!(
            "Error: target path '{}' does not exist.",
            args.target_path.display()
        );
        return ExitCode::FAILURE;
    }

    let plan = if args.target_path.is_file() {
        if !args.json_output {
            println!(
                "Reading cleanup plan from '{}'...",
                args.target_path.display()
            );
        }
        let json_data = match fs::read_to_string(&args.target_path) {
            Ok(data) => data,
            Err(e) => {
                eprintln!(
                    "Error reading plan file '{}': {}",
                    args.target_path.display(),
                    e
                );
                return ExitCode::FAILURE;
            }
        };
        let mut loaded_plan: CleanupPlan = match serde_json::from_str(&json_data) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Error parsing JSON cleanup plan: {}", e);
                return ExitCode::FAILURE;
            }
        };
        if let Some(override_quarantine) = args.quarantine_dir {
            loaded_plan.quarantine_dir = override_quarantine;
        }
        loaded_plan
    } else if args.target_path.is_dir() {
        if !args.json_output {
            println!(
                "Scanning directory and generating plan for '{}'...",
                args.target_path.display()
            );
        }
        let scan_opts = ScanOptions::default();
        let scan_output = match scan_directory(&args.target_path, scan_opts) {
            Ok(output) => output,
            Err(err) => {
                eprintln!("Error scanning directory: {}", err);
                print_root_error_notes(&err);
                return ExitCode::FAILURE;
            }
        };
        let dup_opts = DuplicateOptions {
            include_empty: args.include_empty,
            ..Default::default()
        };
        let dup_output = find_duplicates(&scan_output.entries, &dup_opts);
        let plan_opts = PlanOptions {
            root_dir: args.target_path.clone(),
            policy: args.policy,
            quarantine_dir: args.quarantine_dir,
        };
        match generate_plan(&dup_output.groups, &plan_opts) {
            Ok(plan) => plan,
            Err(err) => {
                eprintln!("Error generating cleanup plan: {}", err);
                return ExitCode::FAILURE;
            }
        }
    } else {
        eprintln!(
            "Error: target '{}' is neither a regular file nor a directory.",
            args.target_path.display()
        );
        return ExitCode::FAILURE;
    };

    let journal_path = match args.custom_journal {
        Some(path) => path,
        None => {
            let default_dir = plan.root_dir.join(".reclaim");
            let _ = fs::create_dir_all(&default_dir);
            default_dir.join("journal.db")
        }
    };

    if let Some(parent) = journal_path.parent() {
        if !parent.exists() {
            let _ = fs::create_dir_all(parent);
        }
    }

    let mut journal = match ActionJournal::open(&journal_path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!(
                "Error opening SQLite action journal at '{}': {}",
                journal_path.display(),
                e
            );
            return ExitCode::FAILURE;
        }
    };

    let exec_options = ExecutionOptions {
        dry_run: args.dry_run,
        halt_on_error: args.halt_on_error,
    };
    let mut executor = SafeExecutor::new(exec_options);

    if args.json_output {
        match executor.execute_plan(&mut journal, &plan) {
            Ok(report) => match serde_json::to_string_pretty(&report) {
                Ok(json_str) => {
                    println!("{}", json_str);
                    if report.failed_actions > 0 {
                        ExitCode::FAILURE
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("Error serializing execution report to JSON: {}", e);
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("Execution failed with storage error: {}", e);
                ExitCode::FAILURE
            }
        }
    } else {
        println!("Reclaim Safe Quarantine Executor v{}", VERSION);
        println!("Target root:        {}", plan.root_dir.display());
        println!("Quarantine folder:  {}", plan.quarantine_dir.display());
        println!("Journal database:   {}", journal_path.display());
        println!(
            "Execution mode:     {}",
            if args.dry_run {
                "DRY-RUN (Pre-validation & simulation; NO files will be moved)"
            } else {
                "LIVE MUTATION (Atomic non-replacing move to quarantine)"
            }
        );
        println!("Actions planned:    {}", plan.actions.len());
        println!(
            "Reclaimable bytes:  {} ({} bytes)",
            format_bytes(plan.summary.total_reclaimable_bytes),
            plan.summary.total_reclaimable_bytes
        );
        println!("==================================================");
        println!();

        if plan.actions.is_empty() {
            println!("No actions to execute. Directory is clean.");
            return ExitCode::SUCCESS;
        }

        println!("Starting execution with real-time TOCTOU precondition validation...");
        println!();

        let report_res =
            executor.execute_plan_with_callback(&mut journal, &plan, |event| match event {
                ExecutionProgressEvent::Validating {
                    source,
                    destination,
                    index,
                    total,
                    ..
                } => {
                    println!(
                        "[{}/{}] Validating preconditions: '{}' -> '{}'",
                        index,
                        total,
                        source.display(),
                        destination.display()
                    );
                }
                ExecutionProgressEvent::Executing {
                    source, dry_run, ..
                } => {
                    if dry_run {
                        println!("  [SIMULATING MOVE] '{}'", source.display());
                    } else {
                        println!("  [MOVING ATOMICALLY] '{}'", source.display());
                    }
                }
                ExecutionProgressEvent::Completed {
                    source,
                    bytes_freed,
                    dry_run,
                    ..
                } => {
                    if dry_run {
                        println!(
                            "  ✔ [DRY RUN VALIDATED] '{}' (would reclaim {})",
                            source.display(),
                            format_bytes(bytes_freed)
                        );
                    } else {
                        println!(
                            "  ✔ [QUARANTINED] '{}' (freed {})",
                            source.display(),
                            format_bytes(bytes_freed)
                        );
                    }
                }
                ExecutionProgressEvent::Failed { source, reason, .. } => {
                    eprintln!("  ✖ [FAILED] '{}': {}", source.display(), reason);
                }
                ExecutionProgressEvent::Skipped { source, reason, .. } => {
                    println!("  - [SKIPPED] '{}': {}", source.display(), reason);
                }
            });

        let report = match report_res {
            Ok(r) => r,
            Err(e) => {
                eprintln!();
                eprintln!("Execution aborted with storage error: {}", e);
                return ExitCode::FAILURE;
            }
        };

        println!();
        print_execution_summary(&report, &journal_path);

        if report.failed_actions > 0 {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

fn print_execution_summary(report: &ExecutionReport, journal_path: &Path) {
    println!("--------------------------------------------------");
    println!("Execution Summary");
    println!("--------------------------------------------------");
    println!(
        "Status:                {}",
        if report.failed_actions == 0 {
            if report.dry_run {
                "Dry Run Completed (Simulation Successful)"
            } else {
                "Completed Successfully"
            }
        } else {
            "Completed with Failures"
        }
    );
    println!("Duration:              {:.2?}", report.elapsed);
    println!("Session ID:            {}", report.session_id);
    println!("Plan ID:               {}", report.plan_id);
    println!("Total actions:         {}", report.total_actions);
    println!("Actions completed:     {}", report.completed_actions);
    println!("Actions failed:        {}", report.failed_actions);
    println!("Actions skipped:       {}", report.skipped_actions);
    println!(
        "Storage reclaimed:     {} ({} bytes)",
        format_bytes(report.bytes_reclaimed),
        report.bytes_reclaimed
    );
    println!("Journal path:          {}", journal_path.display());
    println!("--------------------------------------------------");
    if report.dry_run {
        println!("Dry run mode active: Zero filesystem changes were committed.");
    } else if report.failed_actions == 0 {
        println!("All duplicate files were safely moved to quarantine.");
    } else {
        println!("Warning: Some actions failed. Inspect the journal audit trail for diagnostics.");
    }
}

fn resolve_journal_path(custom: Option<PathBuf>, target_dir: Option<&Path>) -> PathBuf {
    if let Some(path) = custom {
        return path;
    }
    if let Some(dir) = target_dir {
        return dir.join(".reclaim").join("journal.db");
    }
    let cwd_journal = PathBuf::from(".reclaim").join("journal.db");
    if cwd_journal.exists() {
        return cwd_journal;
    }
    cwd_journal
}

fn parse_and_run_undo(raw_args: &[String]) -> ExitCode {
    let mut target_dir = None;
    let mut target_session = None;
    let mut use_last = false;
    let mut dry_run = false;
    let mut custom_journal = None;
    let mut json_output = false;
    let mut halt_on_error = false;

    let mut i = 0;
    while i < raw_args.len() {
        match raw_args[i].as_str() {
            "-h" | "--help" => {
                print_undo_help();
                return ExitCode::SUCCESS;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--last" => {
                use_last = true;
                i += 1;
            }
            "--halt-on-error" => {
                halt_on_error = true;
                i += 1;
            }
            "--json" => {
                json_output = true;
                i += 1;
            }
            "--session" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --session requires a UUID");
                    return ExitCode::FAILURE;
                }
                match SessionId::parse_str(&raw_args[i]) {
                    Ok(sid) => target_session = Some(sid),
                    Err(e) => {
                        eprintln!("Error: invalid session UUID '{}': {}", raw_args[i], e);
                        return ExitCode::FAILURE;
                    }
                }
                i += 1;
            }
            val if val.starts_with("--session=") => {
                let uuid_str = val.trim_start_matches("--session=");
                match SessionId::parse_str(uuid_str) {
                    Ok(sid) => target_session = Some(sid),
                    Err(e) => {
                        eprintln!("Error: invalid session UUID '{}': {}", uuid_str, e);
                        return ExitCode::FAILURE;
                    }
                }
                i += 1;
            }
            "--journal" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --journal requires a path");
                    return ExitCode::FAILURE;
                }
                custom_journal = Some(PathBuf::from(&raw_args[i]));
                i += 1;
            }
            val if val.starts_with("--journal=") => {
                custom_journal = Some(PathBuf::from(val.trim_start_matches("--journal=")));
                i += 1;
            }
            other if !other.starts_with('-') && target_dir.is_none() => {
                target_dir = Some(PathBuf::from(other));
                i += 1;
            }
            other => {
                eprintln!("Error: unrecognized argument '{}'", other);
                print_undo_help();
                return ExitCode::FAILURE;
            }
        }
    }

    if target_session.is_some() && use_last {
        eprintln!("Error: Cannot specify both --session and --last.");
        return ExitCode::FAILURE;
    }

    let journal_path = resolve_journal_path(custom_journal, target_dir.as_deref());
    if !journal_path.exists() {
        eprintln!(
            "Error: SQLite action journal database not found at '{}'.",
            journal_path.display()
        );
        eprintln!("Please specify a valid journal path using --journal <path>.");
        return ExitCode::FAILURE;
    }

    let mut journal = match ActionJournal::open(&journal_path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!(
                "Error opening SQLite journal at '{}': {}",
                journal_path.display(),
                e
            );
            return ExitCode::FAILURE;
        }
    };

    let session_id = match target_session {
        Some(sid) => sid,
        None => {
            let found = match journal.get_last_executed_session() {
                Ok(Some(s)) => Some(s.session_id),
                _ => match journal.get_last_session() {
                    Ok(Some(s)) => Some(s.session_id),
                    _ => None,
                },
            };

            match found {
                Some(sid) => sid,
                None => {
                    eprintln!(
                        "Error: No execution sessions found in journal '{}'.",
                        journal_path.display()
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    let undo_options = UndoOptions {
        dry_run,
        halt_on_error,
    };
    let reverter = SafeReverter::new(undo_options);

    if json_output {
        match reverter.revert_session(&mut journal, session_id) {
            Ok(report) => match serde_json::to_string_pretty(&report) {
                Ok(json_str) => {
                    println!("{}", json_str);
                    if report.actions_conflicted > 0 {
                        ExitCode::FAILURE
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("Failed to serialize undo report to JSON: {}", e);
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("Error executing undo: {}", e);
                ExitCode::FAILURE
            }
        }
    } else {
        println!("Reclaim Safe Conditional Undo v{}", VERSION);
        println!("Target session:     {}", session_id);
        println!("Journal database:   {}", journal_path.display());
        println!(
            "Execution mode:     {}",
            if dry_run {
                "DRY-RUN (Pre-validation & conflict detection; NO files will be moved)"
            } else {
                "LIVE (Atomic restoration to original paths)"
            }
        );
        println!("--------------------------------------------------");
        println!("Validating restoration preconditions and destination vacancies...");
        println!();

        let report_res =
            reverter.revert_session_with_callback(&mut journal, session_id, |event| match event {
                UndoProgressEvent::Validating { .. } => {}
                UndoProgressEvent::Restoring { .. } => {}
                UndoProgressEvent::Restored {
                    source,
                    bytes_restored,
                    dry_run,
                    ..
                } => {
                    if dry_run {
                        println!(
                            "  ✔ [DRY RUN VALIDATED] '{}' ({} to restore)",
                            source.display(),
                            format_bytes(bytes_restored)
                        );
                    } else {
                        println!(
                            "  ✔ [RESTORED] '{}' ({} restored)",
                            source.display(),
                            format_bytes(bytes_restored)
                        );
                    }
                }
                UndoProgressEvent::Conflict { source, reason, .. } => {
                    eprintln!("  ✖ [CONFLICT] '{}': {}", source.display(), reason);
                }
                UndoProgressEvent::Skipped { source, reason, .. } => {
                    println!("  - [SKIPPED] '{}': {}", source.display(), reason);
                }
            });

        let report = match report_res {
            Ok(rep) => rep,
            Err(e) => {
                eprintln!();
                eprintln!("Undo failed: {}", e);
                return ExitCode::FAILURE;
            }
        };

        println!();
        print_undo_summary(&report, &journal_path);

        if report.actions_conflicted > 0 {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

fn print_undo_summary(report: &UndoReport, journal_path: &Path) {
    println!("--------------------------------------------------");
    println!("Undo Summary");
    println!("--------------------------------------------------");
    println!(
        "Status:                {}",
        if report.dry_run {
            "Simulated (Dry Run)"
        } else if report.actions_conflicted == 0 {
            "Completed Successfully"
        } else {
            "Completed with Conflicts"
        }
    );
    println!("Duration:              {:.2?}", report.elapsed);
    if let Some(sid) = report.session_id {
        println!("Session ID:            {}", sid);
    }
    println!("Total actions:         {}", report.total_actions_considered);
    println!("Files restored:        {}", report.actions_restored);
    println!("Conflicts encountered: {}", report.actions_conflicted);
    println!("Actions skipped:       {}", report.actions_skipped);
    println!(
        "Storage restored:      {} ({} bytes)",
        format_bytes(report.bytes_restored),
        report.bytes_restored
    );
    println!("Journal path:          {}", journal_path.display());
    println!("--------------------------------------------------");
    if report.dry_run {
        println!("Dry run mode active: Zero filesystem changes were committed.");
    } else if report.actions_conflicted == 0 {
        println!("All files were safely restored to their original locations.");
    } else {
        println!(
            "Safety Guarantee: Occupied destinations and conflicted files were NOT overwritten."
        );
        println!("Quarantined copies remain safely preserved.");
    }
}

fn parse_and_run_recover(raw_args: &[String]) -> ExitCode {
    let mut target_dir = None;
    let mut custom_journal = None;
    let mut json_output = false;

    let mut i = 0;
    while i < raw_args.len() {
        match raw_args[i].as_str() {
            "-h" | "--help" => {
                print_recover_help();
                return ExitCode::SUCCESS;
            }
            "--json" => {
                json_output = true;
                i += 1;
            }
            "--journal" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --journal requires a path");
                    return ExitCode::FAILURE;
                }
                custom_journal = Some(PathBuf::from(&raw_args[i]));
                i += 1;
            }
            val if val.starts_with("--journal=") => {
                custom_journal = Some(PathBuf::from(val.trim_start_matches("--journal=")));
                i += 1;
            }
            other if !other.starts_with('-') && target_dir.is_none() => {
                target_dir = Some(PathBuf::from(other));
                i += 1;
            }
            other => {
                eprintln!("Error: unrecognized argument '{}'", other);
                print_recover_help();
                return ExitCode::FAILURE;
            }
        }
    }

    let journal_path = resolve_journal_path(custom_journal, target_dir.as_deref());
    if !journal_path.exists() {
        eprintln!(
            "Error: SQLite action journal database not found at '{}'.",
            journal_path.display()
        );
        return ExitCode::FAILURE;
    }

    let mut journal = match ActionJournal::open(&journal_path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!(
                "Error opening SQLite journal at '{}': {}",
                journal_path.display(),
                e
            );
            return ExitCode::FAILURE;
        }
    };

    match reclaim_storage::reconcile_startup_state(&mut journal) {
        Ok(report) => {
            if json_output {
                match serde_json::to_string_pretty(&report) {
                    Ok(json_str) => println!("{}", json_str),
                    Err(e) => {
                        eprintln!("Failed to serialize recovery report to JSON: {}", e);
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                println!(
                    "Reclaim Crash Recovery & Reconciliation Engine v{}",
                    VERSION
                );
                println!("Journal database: {}", journal_path.display());
                println!("--------------------------------------------------");
                if report.is_clean {
                    println!("✔ All previous cleanup sessions completed cleanly.");
                    println!("  No dangling in-flight actions or interrupted sessions found.");
                } else {
                    println!("Crash reconciliation completed:");
                    println!(
                        "  Dangling in-flight actions inspected: {}",
                        report.actions_checked
                    );
                    println!(
                        "  Actions resolved as Completed:       {}",
                        report.actions_resolved_completed
                    );
                    println!(
                        "  Actions resolved as Failed:          {}",
                        report.actions_resolved_failed
                    );
                    println!(
                        "  Interrupted sessions reconciled:     {}",
                        report.sessions_interrupted
                    );
                    if !report.details.is_empty() {
                        println!();
                        println!("Reconciled Actions:");
                        for d in &report.details {
                            println!(
                                "  [{}] Action {} ({})",
                                d.final_state,
                                d.action_id,
                                d.source_path.display()
                            );
                            println!("         {}", d.note);
                        }
                    }
                }
                println!("--------------------------------------------------");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Recovery failed: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn parse_and_run_history(raw_args: &[String]) -> ExitCode {
    let mut target_dir = None;
    let mut target_session = None;
    let mut custom_journal = None;
    let mut json_output = false;

    let mut i = 0;
    while i < raw_args.len() {
        match raw_args[i].as_str() {
            "-h" | "--help" => {
                print_history_help();
                return ExitCode::SUCCESS;
            }
            "--json" => {
                json_output = true;
                i += 1;
            }
            "--session" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --session requires a UUID");
                    return ExitCode::FAILURE;
                }
                match SessionId::parse_str(&raw_args[i]) {
                    Ok(sid) => target_session = Some(sid),
                    Err(e) => {
                        eprintln!("Error: invalid session UUID '{}': {}", raw_args[i], e);
                        return ExitCode::FAILURE;
                    }
                }
                i += 1;
            }
            val if val.starts_with("--session=") => {
                let uuid_str = val.trim_start_matches("--session=");
                match SessionId::parse_str(uuid_str) {
                    Ok(sid) => target_session = Some(sid),
                    Err(e) => {
                        eprintln!("Error: invalid session UUID '{}': {}", uuid_str, e);
                        return ExitCode::FAILURE;
                    }
                }
                i += 1;
            }
            "--journal" => {
                i += 1;
                if i >= raw_args.len() {
                    eprintln!("Error: --journal requires a path");
                    return ExitCode::FAILURE;
                }
                custom_journal = Some(PathBuf::from(&raw_args[i]));
                i += 1;
            }
            val if val.starts_with("--journal=") => {
                custom_journal = Some(PathBuf::from(val.trim_start_matches("--journal=")));
                i += 1;
            }
            other if !other.starts_with('-') && target_dir.is_none() => {
                target_dir = Some(PathBuf::from(other));
                i += 1;
            }
            other => {
                eprintln!("Error: unrecognized argument '{}'", other);
                print_history_help();
                return ExitCode::FAILURE;
            }
        }
    }

    let journal_path = resolve_journal_path(custom_journal, target_dir.as_deref());
    if !journal_path.exists() {
        eprintln!(
            "Error: SQLite action journal database not found at '{}'.",
            journal_path.display()
        );
        return ExitCode::FAILURE;
    }

    let journal = match ActionJournal::open(&journal_path) {
        Ok(j) => j,
        Err(e) => {
            eprintln!(
                "Error opening SQLite journal at '{}': {}",
                journal_path.display(),
                e
            );
            return ExitCode::FAILURE;
        }
    };

    if let Some(session_id) = target_session {
        match journal.fetch_session_history(session_id) {
            Ok(session) => {
                let actions = journal
                    .list_actions_for_session(session_id)
                    .unwrap_or_default();
                if json_output {
                    let json_val = serde_json::json!({
                        "session": session,
                        "actions": actions,
                    });
                    println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
                } else {
                    println!("Session Details: {}", session_id);
                    println!("Target root:     {}", session.target_root.display());
                    println!("Status:          {}", session.status);
                    println!("Keeper policy:   {}", session.keeper_policy);
                    println!("Plans count:     {}", session.plan_ids.len());
                    println!("Actions count:   {}", actions.len());
                    println!("--------------------------------------------------");
                    for a in &actions {
                        println!(
                            "  [{:<9}] Action {} ({})",
                            a.state,
                            a.action_id,
                            a.source_path.display()
                        );
                        println!("            Quarantine: {}", a.quarantine_path.display());
                        println!("            Keeper:     {}", a.keeper_path.display());
                    }
                    println!("--------------------------------------------------");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error retrieving session '{}': {}", session_id, e);
                ExitCode::FAILURE
            }
        }
    } else {
        match journal.list_all_sessions() {
            Ok(sessions) => {
                let mut summaries = Vec::new();
                for s in &sessions {
                    if let Ok(sum) = journal.fetch_session_summary(s.session_id) {
                        summaries.push(sum);
                    }
                }

                if json_output {
                    println!("{}", serde_json::to_string_pretty(&summaries).unwrap());
                } else {
                    println!("Reclaim Cleanup Session History v{}", VERSION);
                    println!("Journal database: {}", journal_path.display());
                    println!("--------------------------------------------------");
                    if summaries.is_empty() {
                        println!("No execution sessions recorded in journal.");
                    } else {
                        println!(
                            "{:<36}  {:<11}  {:<14}  {:<13}  TARGET ROOT",
                            "SESSION ID", "STATUS", "ACTIONS (C/R/F)", "QUARANTINED"
                        );
                        println!(
                            "{:-<36}  {:-<11}  {:-<14}  {:-<13}  {:-<20}",
                            "", "", "", "", ""
                        );
                        for s in &summaries {
                            let actions_summary = format!(
                                "{}/{}/{}",
                                s.completed_actions, s.reverted_actions, s.failed_actions
                            );
                            println!(
                                "{:<36}  {:<11}  {:<14}  {:<13}  {}",
                                s.session_id,
                                s.status,
                                actions_summary,
                                format_bytes(s.active_quarantined_bytes),
                                s.target_root.display()
                            );
                        }
                    }
                    println!("--------------------------------------------------");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error querying session history: {}", e);
                ExitCode::FAILURE
            }
        }
    }
}

fn print_plan_human(plan: &CleanupPlan) {
    println!("Quarantine directory: {}", plan.quarantine_dir.display());
    println!("==================================================");
    println!();

    if plan.keepers.is_empty() {
        println!("No duplicate files found in target directory.");
        println!("Zero cleanup actions planned.");
        println!();
    } else {
        println!(
            "Evaluated {} duplicate group(s) across {} candidate file(s):",
            plan.summary.duplicate_groups_count, plan.summary.total_duplicate_files
        );
        println!();

        for (idx, keeper) in plan.keepers.iter().enumerate() {
            println!("--- Duplicate Group #{} ---", idx + 1);
            println!("  [PROTECTED KEEPER]");
            println!("  Path:      {}", keeper.path.display());
            println!(
                "  Size:      {} ({} bytes)",
                format_bytes(keeper.size),
                keeper.size
            );
            println!("  Hash:      {}", keeper.hash);
            if let Some(fid) = keeper.file_id {
                println!("  FileId:    {}", fid);
            }
            println!("  Rationale: {}", keeper.rationale);

            if !keeper.hard_link_aliases.is_empty() {
                println!(
                    "  Protected Keeper Aliases ({} hard links sharing keeper inode):",
                    keeper.hard_link_aliases.len()
                );
                for alias in &keeper.hard_link_aliases {
                    println!("    - {}", alias.display());
                }
            }

            // Find proposed actions targeting files in this group (associated with this keeper)
            let group_actions: Vec<_> = plan
                .actions
                .iter()
                .filter(|a| a.keeper_path == keeper.path)
                .collect();

            if group_actions.is_empty() {
                println!("  [NO ACTIONS PROPOSED] (Pure hard-link group; 0 reclaimable bytes)");
            } else {
                println!(
                    "  [PROPOSED QUARANTINE ACTIONS] ({} action{})",
                    group_actions.len(),
                    if group_actions.len() == 1 { "" } else { "s" }
                );
                for (act_idx, action) in group_actions.iter().enumerate() {
                    println!("    Action #{}:", act_idx + 1);
                    println!("      Source:       {}", action.source_path.display());
                    println!("      Destination:  {}", action.quarantine_path.display());
                    println!("      Preconditions:");
                    println!(
                        "        Size:       {} bytes",
                        action.preconditions.expected_size
                    );
                    println!("        Hash:       {}", action.preconditions.expected_hash);
                    if let Some(fid) = action.preconditions.source_file_id {
                        println!("        FileId:     {}", fid);
                    }
                    println!("      Rationale:    {}", action.rationale);
                }
            }
            println!();
        }
    }

    print_plan_summary(&plan.summary);
}

fn print_plan_summary(summary: &PlanSummary) {
    println!("--------------------------------------------------");
    println!("Cleanup Plan Summary");
    println!("--------------------------------------------------");
    println!(
        "Duplicate groups evaluated:  {}",
        summary.duplicate_groups_count
    );
    println!(
        "Total candidate files:       {}",
        summary.total_duplicate_files
    );
    println!("Protected keeper files:      {}", summary.protected_keepers);
    println!("Proposed quarantine actions: {}", summary.planned_actions);
    println!(
        "Conservative reclaimable:    {} ({} bytes)",
        format_bytes(summary.total_reclaimable_bytes),
        summary.total_reclaimable_bytes
    );
    println!(
        "Files left untouched:        {}",
        summary.files_left_untouched
    );
    println!("--------------------------------------------------");
    println!("Status: Ready for user review. No files were modified.");
}

fn print_summary(summary: &ScanSummary) {
    println!("Scan Summary");
    println!("--------------------------------------------------");
    println!(
        "Status:                 {}",
        if summary.was_cancelled {
            "Cancelled"
        } else {
            "Completed"
        }
    );
    println!("Duration:               {:.2?}", summary.elapsed);
    println!("Directories visited:    {}", summary.directories_visited);
    println!("Files discovered:       {}", summary.files_discovered);
    println!(
        "Total file bytes:       {} ({} bytes)",
        format_bytes(summary.total_file_bytes),
        summary.total_file_bytes
    );
    println!("Symlinks skipped:       {}", summary.symlinks_skipped);
    println!("Other skipped:          {}", summary.other_skipped);
    println!("Errors encountered:     {}", summary.errors_count);
    println!("--------------------------------------------------");
}

fn print_duplicate_summary(summary: &DuplicateSummary) {
    println!("Duplicate Detection Summary");
    println!("--------------------------------------------------");
    println!(
        "Status:                      {}",
        if summary.was_cancelled {
            "Cancelled"
        } else {
            "Completed"
        }
    );
    println!("Duration:                    {:.2?}", summary.elapsed);
    println!(
        "Total files evaluated:       {}",
        summary.total_files_evaluated
    );
    println!(
        "Empty (0-byte) files ignored: {}",
        summary.empty_files_skipped
    );
    println!(
        "Size-matched candidates:     {}",
        summary.size_matched_candidates
    );
    println!(
        "Prefix-matched candidates:   {}",
        summary.prefix_matched_candidates
    );
    println!(
        "Exact duplicate groups:      {}",
        summary.duplicate_groups_count
    );
    println!(
        "Total duplicate files:       {}",
        summary.duplicate_files_count
    );
    println!(
        "Distinct physical copies:    {}",
        summary.distinct_physical_copies
    );
    println!(
        "Hard-link aliases:           {}",
        summary.hard_link_aliases_count
    );
    println!(
        "Conservative storage savings: {} ({} bytes)",
        format_bytes(summary.total_reclaimable_bytes),
        summary.total_reclaimable_bytes
    );
    println!("--------------------------------------------------");
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
    }
}
