use reclaim_core::{
    find_duplicates, generate_plan, scan_directory, CleanupPlan, DuplicateOptions,
    DuplicateSummary, KeeperPolicy, PlanOptions, PlanSummary, RootScanError, ScanOptions,
    ScanSummary,
};
use std::env;
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
    println!("    help                              Print this help information");
    println!("    version                           Print version");
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
