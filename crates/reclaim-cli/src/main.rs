use reclaim_core::{scan_directory, RootScanError, ScanOptions, ScanSummary};
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
    println!("    scan <directory>    Recursively scan a directory in read-only mode");
    println!("    help                Print this help information");
    println!("    version             Print version");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help          Print help");
    println!("    -v, --version       Print version");
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
            match err {
                RootScanError::IsSymbolicLink(_) => {
                    eprintln!("Note: Reclaim refuses to scan root symlinks to prevent unexpected traversal.");
                }
                RootScanError::IsReparsePoint(_) => {
                    eprintln!(
                        "Note: Reclaim refuses to scan root reparse points / junctions for safety."
                    );
                }
                _ => {}
            }
            ExitCode::FAILURE
        }
    }
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
