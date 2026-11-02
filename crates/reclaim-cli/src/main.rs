use reclaim_core::{
    find_duplicates, scan_directory, DuplicateOptions, DuplicateOutput, DuplicateSummary,
    RootScanError, ScanOptions, ScanSummary,
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
    println!("    help                              Print this help information");
    println!("    version                           Print version");
    println!();
    println!("DUPLICATE OPTIONS:");
    println!("    --include-empty                   Include 0-byte files in duplicate detection");
    println!();
    println!("GLOBAL OPTIONS:");
    println!("    -h, --help                        Print help");
    println!("    -v, --version                     Print version");
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

            if !output.errors.is_empty() {
                println!();
                println!("Non-fatal Errors ({}):", output.errors.len());
                for (idx, err) in output.errors.iter().enumerate().take(10) {
                    println!("  {:2}. {}", idx + 1, err);
                }
                if output.errors.len() > 10 {
                    println!("  ... and {} more errors.", output.errors.len() - 10);
                }
            }

            if !output.skipped.is_empty() {
                println!();
                println!("Skipped Entries ({}):", output.skipped.len());
                for (idx, skip) in output.skipped.iter().enumerate().take(10) {
                    println!("  {:2}. [{}] {}", idx + 1, skip.reason, skip.path.display());
                }
                if output.skipped.len() > 10 {
                    println!(
                        "  ... and {} more skipped entries.",
                        output.skipped.len() - 10
                    );
                }
            }

            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("Scan aborted: {}", err);
            print_root_error_notes(&err);
            ExitCode::FAILURE
        }
    }
}

fn run_duplicates(target_path: &Path, include_empty: bool) -> ExitCode {
    println!("Reclaim Duplicate Detection v{}", VERSION);
    println!("Target: {}", target_path.display());
    println!("Step 1: Scanning directory in read-only mode...");
    println!();

    let scan_options = ScanOptions::default();
    let scan_output = match scan_directory(target_path, scan_options) {
        Ok(output) => output,
        Err(err) => {
            eprintln!("Scan aborted: {}", err);
            print_root_error_notes(&err);
            return ExitCode::FAILURE;
        }
    };

    println!(
        "Discovered {} regular files ({}) across {} directories in {:.2?}.",
        scan_output.summary.files_discovered,
        format_bytes(scan_output.summary.total_file_bytes),
        scan_output.summary.directories_visited,
        scan_output.summary.elapsed
    );

    if scan_output.entries.is_empty() {
        println!("No regular files found to analyze.");
        return ExitCode::SUCCESS;
    }

    println!("Step 2: Detecting exact duplicate files with two-stage BLAKE3 hashing...");
    println!();

    let dedup_options = DuplicateOptions {
        include_empty,
        ..Default::default()
    };

    let dedup_output = find_duplicates(&scan_output.entries, &dedup_options);

    print_duplicate_results(&dedup_output);

    if !dedup_output.errors.is_empty() {
        println!();
        println!("Hashing Errors ({}):", dedup_output.errors.len());
        for (idx, err) in dedup_output.errors.iter().enumerate().take(10) {
            println!("  {:2}. {}", idx + 1, err);
        }
        if dedup_output.errors.len() > 10 {
            println!(
                "  ... and {} more hashing errors.",
                dedup_output.errors.len() - 10
            );
        }
    }

    ExitCode::SUCCESS
}

fn print_duplicate_results(output: &DuplicateOutput) {
    if output.groups.is_empty() {
        println!("No exact duplicate files found.");
    } else {
        println!(
            "Identified {} duplicate group(s) across {} files:",
            output.groups.len(),
            output.summary.duplicate_files_count
        );
        println!();

        for (idx, group) in output.groups.iter().enumerate() {
            println!(
                "--------------------------------------------------------------------------------"
            );
            println!(
                "Group {} [BLAKE3: {} | File Size: {} | Reclaimable: {}]",
                idx + 1,
                group.hash,
                format_bytes(group.file_size),
                format_bytes(group.reclaimable_bytes)
            );

            if group.has_hard_links() {
                println!(
                    "  {} physical storage copies, {} total paths (contains hard-link aliases):",
                    group.distinct_physical_copies,
                    group.file_count()
                );
                for (copy_idx, copy) in group.physical_copies.iter().enumerate() {
                    let id_str = match &copy.file_id {
                        Some(fid) => format!(" [{}]", fid),
                        None => String::new(),
                    };
                    println!("  Physical Copy {}{}:", copy_idx + 1, id_str);
                    for (path_idx, path) in copy.paths.iter().enumerate() {
                        let alias_tag = if path_idx > 0 {
                            " (hard link alias)"
                        } else {
                            ""
                        };
                        println!("    {}{}", path.display(), alias_tag);
                    }
                }
            } else {
                for entry in &group.entries {
                    println!("  {}", entry.path.display());
                }
            }
        }
        println!(
            "--------------------------------------------------------------------------------"
        );
    }

    println!();
    print_duplicate_summary(&output.summary);
}

fn print_summary(summary: &ScanSummary) {
    println!("Scan Results");
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
