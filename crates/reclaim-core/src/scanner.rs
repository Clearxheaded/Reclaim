use crate::entry::{FileEntry, SkipReason, SkippedEntry};
use crate::error::{RootScanError, ScanError};
use crate::file_id::FileId;
use crate::platform;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A thread-safe cancellation handle to request an early exit from an ongoing scan.
#[derive(Clone, Default, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a new, uncancelled token.
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Requests that any scan monitoring this token abort traversal as soon as possible.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Returns `true` if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Periodic progress notification payload reported during directory traversal.
#[derive(Debug, Clone)]
pub struct ScanProgress {
    /// The directory currently being examined.
    pub current_directory: PathBuf,
    /// Total number of directories entered so far.
    pub directories_visited: u64,
    /// Total number of regular files discovered so far.
    pub files_discovered: u64,
    /// Aggregate byte size of all discovered regular files.
    pub total_file_bytes: u64,
}

/// Callback type invoked with periodic scan progress information.
pub type ProgressCallback = Box<dyn Fn(&ScanProgress) + Send + Sync>;

/// Configuration options controlling scan execution.
#[derive(Default)]
pub struct ScanOptions {
    /// Optional cancellation handle to abort long-running scans.
    pub cancellation_token: Option<CancellationToken>,
    /// Optional callback invoked as the scanner traverses directories.
    pub progress_callback: Option<ProgressCallback>,
}

/// High-level summary metrics produced at the end of a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    /// The root path that was requested for scanning.
    pub target_path: PathBuf,
    /// Total count of successfully visited directories.
    pub directories_visited: u64,
    /// Total count of regular files identified.
    pub files_discovered: u64,
    /// Aggregate size in bytes of all regular files discovered.
    pub total_file_bytes: u64,
    /// Count of symbolic links detected and deliberately skipped.
    pub symlinks_skipped: u64,
    /// Count of other skipped entries (reparse points, sockets, FIFOs, devices).
    pub other_skipped: u64,
    /// Count of non-fatal errors encountered during traversal.
    pub errors_count: u64,
    /// Wall-clock duration of the scan.
    pub elapsed: Duration,
    /// Whether the scan was cancelled before traversing all directories.
    pub was_cancelled: bool,
}

/// The complete output of a filesystem scan operation.
#[derive(Debug, Clone)]
pub struct ScanOutput {
    /// All discovered regular files with their collected metadata.
    pub entries: Vec<FileEntry>,
    /// Summary metrics for the scan.
    pub summary: ScanSummary,
    /// Non-fatal errors encountered while reading directories or inspecting entries.
    pub errors: Vec<ScanError>,
    /// Entries skipped for safety or platform-specific reasons.
    pub skipped: Vec<SkippedEntry>,
}

/// Recursively scans `path` in read-only mode using the provided `options`.
///
/// # Safety and Invariants
/// - Never modifies, moves, or deletes user files.
/// - Never reads file data content.
/// - Never follows symbolic links.
/// - Never traverses Windows junctions or reparse points.
/// - Uses an explicit heap-allocated worklist to prevent call-stack overflows.
/// - Reports permission errors and missing entries without aborting the entire scan.
pub fn scan_directory(
    path: impl AsRef<Path>,
    options: ScanOptions,
) -> Result<ScanOutput, RootScanError> {
    let root = path.as_ref();
    let start_time = Instant::now();

    // 1. Validate the root path using symlink_metadata (never follow symlinks on root).
    let root_meta = match fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                return Err(RootScanError::NotFound(root.to_path_buf()));
            }
            return Err(RootScanError::Io {
                path: root.to_path_buf(),
                source: err,
            });
        }
    };

    let root_file_type = root_meta.file_type();

    if root_file_type.is_symlink() {
        return Err(RootScanError::IsSymbolicLink(root.to_path_buf()));
    }

    if platform::is_reparse_point(&root_file_type) {
        return Err(RootScanError::IsReparsePoint(root.to_path_buf()));
    }

    if !root_file_type.is_dir() {
        return Err(RootScanError::NotADirectory(root.to_path_buf()));
    }

    let mut entries = Vec::new();
    let mut errors = Vec::new();
    let mut skipped = Vec::new();

    let mut directories_visited: u64 = 0;
    let mut files_discovered: u64 = 0;
    let mut total_file_bytes: u64 = 0;
    let mut symlinks_skipped: u64 = 0;
    let mut other_skipped: u64 = 0;
    let mut was_cancelled = false;

    // 2. Iterative Depth-First Search traversal via explicit stack.
    let mut worklist: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(current_dir) = worklist.pop() {
        // Check for cancellation before entering the directory.
        if let Some(ref token) = options.cancellation_token {
            if token.is_cancelled() {
                was_cancelled = true;
                break;
            }
        }

        let read_dir_result = fs::read_dir(&current_dir);
        let read_dir_iter = match read_dir_result {
            Ok(iter) => iter,
            Err(err) => {
                errors.push(ScanError::new(current_dir, err));
                continue;
            }
        };

        directories_visited += 1;

        if let Some(ref cb) = options.progress_callback {
            cb(&ScanProgress {
                current_directory: current_dir.clone(),
                directories_visited,
                files_discovered,
                total_file_bytes,
            });
        }

        for entry_result in read_dir_iter {
            // Check cancellation per directory entry.
            if let Some(ref token) = options.cancellation_token {
                if token.is_cancelled() {
                    was_cancelled = true;
                    break;
                }
            }

            let entry = match entry_result {
                Ok(e) => e,
                Err(err) => {
                    errors.push(ScanError::new(current_dir.clone(), err));
                    continue;
                }
            };

            let entry_path = entry.path();

            // Check file type without following symlinks.
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(err) => {
                    errors.push(ScanError::new(entry_path, err));
                    continue;
                }
            };

            // Invariant: Never follow symbolic links.
            if file_type.is_symlink() {
                skipped.push(SkippedEntry {
                    path: entry_path,
                    reason: SkipReason::SymbolicLink,
                });
                symlinks_skipped += 1;
                continue;
            }

            // Invariant: Never traverse Windows reparse points (junctions, volume mount points).
            if platform::is_reparse_point(&file_type) {
                skipped.push(SkippedEntry {
                    path: entry_path,
                    reason: SkipReason::ReparsePoint,
                });
                other_skipped += 1;
                continue;
            }

            // Skip special file types (FIFOs, sockets, character/block devices).
            if let Some(special_reason) = platform::classify_special_type(&file_type) {
                skipped.push(SkippedEntry {
                    path: entry_path,
                    reason: special_reason,
                });
                other_skipped += 1;
                continue;
            }

            if file_type.is_dir() {
                worklist.push(entry_path);
                continue;
            }

            if file_type.is_file() {
                // Fetch metadata without following symlinks.
                let meta = match fs::symlink_metadata(&entry_path) {
                    Ok(m) => m,
                    Err(err) => {
                        errors.push(ScanError::new(entry_path, err));
                        continue;
                    }
                };

                // Double check symlink status in case of filesystem race condition.
                if meta.file_type().is_symlink() {
                    skipped.push(SkippedEntry {
                        path: entry_path,
                        reason: SkipReason::SymbolicLink,
                    });
                    symlinks_skipped += 1;
                    continue;
                }

                let size = meta.len();
                let modified = meta.modified().ok();
                let file_id = FileId::from_path_and_metadata(&entry_path, &meta);
                let readonly = meta.permissions().readonly();

                files_discovered += 1;
                total_file_bytes = total_file_bytes.saturating_add(size);

                entries.push(FileEntry {
                    path: entry_path,
                    size,
                    modified,
                    file_id,
                    readonly,
                });
                continue;
            }

            // Unrecognized file type fallback.
            skipped.push(SkippedEntry {
                path: entry_path,
                reason: SkipReason::UnknownFileType,
            });
            other_skipped += 1;
        }

        if was_cancelled {
            break;
        }
    }

    let summary = ScanSummary {
        target_path: root.to_path_buf(),
        directories_visited,
        files_discovered,
        total_file_bytes,
        symlinks_skipped,
        other_skipped,
        errors_count: errors.len() as u64,
        elapsed: start_time.elapsed(),
        was_cancelled,
    };

    Ok(ScanOutput {
        entries,
        summary,
        errors,
        skipped,
    })
}
