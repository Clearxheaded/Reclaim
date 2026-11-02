//! # Duplicate Detection Module
//!
//! Provides two-stage BLAKE3 content hashing, file size grouping, and
//! hard-link-aware storage savings calculations for Reclaim (Milestone 2).
//!
//! ## Detection Pipeline
//! 1. **Candidate Grouping by Size**: Group files by exact byte length. Files with unique
//!    sizes are filtered out immediately without touching disk contents or computing hashes.
//! 2. **Stage 1 — Prefix Hashing**: Hash the first 16 KiB (or file size if smaller) of
//!    size-matched candidates. Files differing in their prefix are immediately eliminated.
//!    For files <= 16 KiB, the prefix hash is identical to the full BLAKE3 hash, bypassing Stage 2.
//! 3. **Stage 2 — Full Content Hashing**: Compute the full BLAKE3 hash using streaming I/O
//!    with bounded buffer sizes (64 KiB), verifying candidates whose prefix hashes matched.
//! 4. **Hard-Link Awareness**: Files sharing the exact same `FileId` (same inode on Unix or
//!    volume/file index on Windows) share physical disk blocks. They are tracked as hard-link
//!    aliases and do not inflate conservative reclaimable storage savings.

use crate::entry::FileEntry;
use crate::file_id::FileId;
use crate::scanner::CancellationToken;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Default byte size for Stage 1 prefix hashing (16 KiB).
pub const DEFAULT_PREFIX_SIZE: usize = 16 * 1024;

/// Default buffer size for Stage 2 streaming full hashing (64 KiB).
pub const HASH_BUFFER_SIZE: usize = 64 * 1024;

/// Non-fatal error encountered on a specific file during duplicate detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateFileError {
    /// Path to the file that encountered an error.
    pub path: PathBuf,
    /// Descriptive error message.
    pub message: String,
}

impl std::fmt::Display for DuplicateFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// The current stage of duplicate detection reported in progress callbacks.
#[derive(Debug, Clone)]
pub enum DuplicateStage {
    /// Grouping files by size.
    GroupingBySize,
    /// Stage 1: computing prefix hashes on size-matched candidate files.
    PrefixHashing {
        /// 1-based index of the candidate file currently being processed.
        current_file: u64,
        /// Total number of candidates in this stage.
        total_candidates: u64,
        /// Path of the file currently being processed.
        current_path: PathBuf,
    },
    /// Stage 2: computing full streaming BLAKE3 hashes on prefix-matched candidate files.
    FullHashing {
        /// 1-based index of the candidate file currently being processed.
        current_file: u64,
        /// Total number of candidates in this stage.
        total_candidates: u64,
        /// Path of the file currently being processed.
        current_path: PathBuf,
    },
}

/// Progress notification payload reported during duplicate detection.
#[derive(Debug, Clone)]
pub struct DuplicateProgress {
    /// Detailed stage information.
    pub stage: DuplicateStage,
}

/// Callback type for duplicate detection progress updates.
pub type DuplicateProgressCallback = Box<dyn Fn(&DuplicateProgress) + Send + Sync>;

/// Configuration options controlling duplicate detection.
#[derive(Default)]
pub struct DuplicateOptions {
    /// Byte limit for Stage 1 prefix hashing (default: 16 KiB / 16,384 bytes).
    pub prefix_size: usize,
    /// Whether to include 0-byte files in duplicate detection.
    ///
    /// When `false` (the default), 0-byte files are ignored because they occupy 0 bytes
    /// of storage and cleaning them yields zero reclaimable space.
    pub include_empty: bool,
    /// Optional cancellation handle to abort detection early.
    pub cancellation_token: Option<CancellationToken>,
    /// Optional callback for progress reporting.
    pub progress_callback: Option<DuplicateProgressCallback>,
}

/// A set of file paths that point to the exact same physical storage blocks (hard links).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalCopy {
    /// Filesystem identity (`st_dev`/`st_ino` on Unix, volume/file index on Windows), if available.
    pub file_id: Option<FileId>,
    /// All paths known to share this physical file identity.
    pub paths: Vec<PathBuf>,
}

/// A verified group of files sharing identical content (same BLAKE3 hash and byte length).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    /// Exact BLAKE3 content hash.
    pub hash: blake3::Hash,
    /// Size of each file in this group in bytes.
    pub file_size: u64,
    /// All file entries in this duplicate group.
    pub entries: Vec<FileEntry>,
    /// Physical storage copies grouped by `FileId`.
    /// Multiple paths under a single `PhysicalCopy` are hard links to the same physical blocks.
    pub physical_copies: Vec<PhysicalCopy>,
    /// Count of distinct physical storage copies (`physical_copies.len()`).
    pub distinct_physical_copies: usize,
    /// Conservative estimate of potential storage savings if all but one physical copy were reclaimed.
    ///
    /// Calculated as `(distinct_physical_copies - 1) * file_size` when `distinct_physical_copies > 1`,
    /// or `0` if all paths in the group are hard links to each other or 0-byte files.
    pub reclaimable_bytes: u64,
}

impl DuplicateGroup {
    /// Returns `true` if this group contains hard-link aliases (more paths than physical copies).
    pub fn has_hard_links(&self) -> bool {
        self.entries.len() > self.distinct_physical_copies
    }

    /// Returns `true` if all paths in this group point to the same physical file (0 reclaimable bytes).
    pub fn is_pure_hard_links(&self) -> bool {
        self.distinct_physical_copies <= 1 && self.entries.len() > 1
    }

    /// Returns the total count of file paths in this group.
    pub fn file_count(&self) -> usize {
        self.entries.len()
    }
}

/// High-level metrics summarizing duplicate detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateSummary {
    /// Total input files evaluated.
    pub total_files_evaluated: u64,
    /// Number of 0-byte files ignored.
    pub empty_files_skipped: u64,
    /// Number of candidate files that qualified for Stage 1 prefix hashing.
    pub size_matched_candidates: u64,
    /// Number of candidate files that qualified for Stage 2 full hashing.
    pub prefix_matched_candidates: u64,
    /// Number of duplicate groups discovered.
    pub duplicate_groups_count: usize,
    /// Total number of duplicate file paths across all groups.
    pub duplicate_files_count: usize,
    /// Total number of distinct physical file copies across all groups.
    pub distinct_physical_copies: usize,
    /// Total number of hard-link alias paths across all groups.
    pub hard_link_aliases_count: usize,
    /// Conservative potential storage savings across all duplicate groups in bytes.
    pub total_reclaimable_bytes: u64,
    /// Wall-clock duration of duplicate detection.
    pub elapsed: Duration,
    /// Whether detection was interrupted by cancellation.
    pub was_cancelled: bool,
}

/// The complete output of duplicate detection.
#[derive(Debug, Clone)]
pub struct DuplicateOutput {
    /// Discovered duplicate groups, sorted by reclaimable bytes descending, then size descending.
    pub groups: Vec<DuplicateGroup>,
    /// Summary metrics.
    pub summary: DuplicateSummary,
    /// Non-fatal errors encountered during hashing.
    pub errors: Vec<DuplicateFileError>,
}

/// Internal record for a candidate file after Stage 1 prefix hashing.
struct PrefixCandidate<'a> {
    entry: &'a FileEntry,
    prefix_hash: blake3::Hash,
    is_full_hash: bool,
}

/// Finds exact duplicate files among the provided `entries` using the given `options`.
///
/// # Guarantees
/// - **Read-only**: Never modifies, moves, or deletes files.
/// - **Safe & Cryptographic**: Uses BLAKE3 content hashing. Filename similarity is never used.
/// - **Hard-link aware**: Differentiates distinct physical files from hard links sharing physical storage.
/// - **Bounded memory**: Streams large files in chunks without reading entire files into RAM.
/// - **Responsive**: Checks cancellation periodically and reports progress via callbacks.
pub fn find_duplicates(entries: &[FileEntry], options: &DuplicateOptions) -> DuplicateOutput {
    let start_time = Instant::now();
    let prefix_size = if options.prefix_size == 0 {
        DEFAULT_PREFIX_SIZE
    } else {
        options.prefix_size
    };

    let mut errors: Vec<DuplicateFileError> = Vec::new();
    let mut empty_files_skipped = 0u64;
    let mut was_cancelled = false;

    // Check cancellation before starting
    if let Some(token) = &options.cancellation_token {
        if token.is_cancelled() {
            return DuplicateOutput {
                groups: Vec::new(),
                summary: DuplicateSummary {
                    total_files_evaluated: entries.len() as u64,
                    empty_files_skipped: 0,
                    size_matched_candidates: 0,
                    prefix_matched_candidates: 0,
                    duplicate_groups_count: 0,
                    duplicate_files_count: 0,
                    distinct_physical_copies: 0,
                    hard_link_aliases_count: 0,
                    total_reclaimable_bytes: 0,
                    elapsed: start_time.elapsed(),
                    was_cancelled: true,
                },
                errors,
            };
        }
    }

    if let Some(cb) = &options.progress_callback {
        cb(&DuplicateProgress {
            stage: DuplicateStage::GroupingBySize,
        });
    }

    // Step 1: Candidate Grouping by Size
    let mut size_groups: HashMap<u64, Vec<&FileEntry>> = HashMap::new();
    for entry in entries {
        if entry.size == 0 && !options.include_empty {
            empty_files_skipped += 1;
            continue;
        }
        size_groups.entry(entry.size).or_default().push(entry);
    }

    // Filter out unique sizes (size groups with < 2 files)
    let mut candidate_size_groups: Vec<(u64, Vec<&FileEntry>)> = size_groups
        .into_iter()
        .filter(|(_, files)| files.len() >= 2)
        .collect();

    // Sort descending by size for deterministic processing and prioritizing large files
    candidate_size_groups.sort_by_key(|a| std::cmp::Reverse(a.0));

    let size_matched_candidates: u64 = candidate_size_groups
        .iter()
        .map(|(_, files)| files.len() as u64)
        .sum();

    // Cache prefix hashes by FileId to avoid redundant I/O for hard links
    let mut file_id_prefix_cache: HashMap<FileId, (blake3::Hash, bool)> = HashMap::new();
    let mut prefix_candidates_processed = 0u64;

    // Candidates that qualify for Stage 2 (or are already verified exact duplicates)
    // Key: (file_size, prefix_hash)
    let mut prefix_matched_groups: HashMap<(u64, blake3::Hash), Vec<PrefixCandidate>> =
        HashMap::new();

    // Step 2: Stage 1 — Prefix Hashing
    'size_loop: for (file_size, group_files) in candidate_size_groups {
        let mut local_prefix_map: HashMap<blake3::Hash, Vec<PrefixCandidate>> = HashMap::new();

        for entry in group_files {
            if let Some(token) = &options.cancellation_token {
                if token.is_cancelled() {
                    was_cancelled = true;
                    break 'size_loop;
                }
            }

            prefix_candidates_processed += 1;
            if let Some(cb) = &options.progress_callback {
                cb(&DuplicateProgress {
                    stage: DuplicateStage::PrefixHashing {
                        current_file: prefix_candidates_processed,
                        total_candidates: size_matched_candidates,
                        current_path: entry.path.clone(),
                    },
                });
            }

            // Check if we already hashed this physical inode/FileId
            let prefix_result = if let Some(fid) = entry.file_id {
                if let Some(&(cached_hash, cached_is_full)) = file_id_prefix_cache.get(&fid) {
                    Some((cached_hash, cached_is_full))
                } else {
                    None
                }
            } else {
                None
            };

            let (hash, is_full) = match prefix_result {
                Some(cached) => cached,
                None => match read_prefix_hash(&entry.path, entry.size, prefix_size) {
                    Ok(res) => {
                        if let Some(fid) = entry.file_id {
                            file_id_prefix_cache.insert(fid, res);
                        }
                        res
                    }
                    Err(err) => {
                        errors.push(DuplicateFileError {
                            path: entry.path.clone(),
                            message: format!("Failed to read prefix: {}", err),
                        });
                        continue;
                    }
                },
            };

            local_prefix_map
                .entry(hash)
                .or_default()
                .push(PrefixCandidate {
                    entry,
                    prefix_hash: hash,
                    is_full_hash: is_full,
                });
        }

        // Only retain prefix matches with >= 2 candidates
        for (prefix_hash, candidates) in local_prefix_map {
            if candidates.len() >= 2 {
                prefix_matched_groups.insert((file_size, prefix_hash), candidates);
            }
        }
    }

    let prefix_matched_candidates: u64 = prefix_matched_groups
        .values()
        .map(|candidates| candidates.len() as u64)
        .sum();

    // Cache full hashes by FileId to avoid redundant streaming I/O for hard links
    let mut file_id_full_cache: HashMap<FileId, blake3::Hash> = HashMap::new();
    let mut full_candidates_processed = 0u64;

    // Groups verified by exact full hash
    // Key: (file_size, full_hash)
    let mut verified_groups: HashMap<(u64, blake3::Hash), Vec<&FileEntry>> = HashMap::new();

    // Step 3: Stage 2 — Full Content Hashing
    if !was_cancelled {
        'prefix_loop: for ((file_size, _prefix_hash), candidates) in prefix_matched_groups {
            // Check if candidates are already fully hashed (files <= prefix_size)
            let all_fully_hashed = candidates.iter().all(|c| c.is_full_hash);

            if all_fully_hashed {
                full_candidates_processed += candidates.len() as u64;
                for candidate in candidates {
                    verified_groups
                        .entry((file_size, candidate.prefix_hash))
                        .or_default()
                        .push(candidate.entry);
                }
                continue;
            }

            // Files larger than prefix_size: compute streaming full BLAKE3 hash
            let mut local_full_map: HashMap<blake3::Hash, Vec<&FileEntry>> = HashMap::new();

            for candidate in candidates {
                if let Some(token) = &options.cancellation_token {
                    if token.is_cancelled() {
                        was_cancelled = true;
                        break 'prefix_loop;
                    }
                }

                full_candidates_processed += 1;
                if let Some(cb) = &options.progress_callback {
                    cb(&DuplicateProgress {
                        stage: DuplicateStage::FullHashing {
                            current_file: full_candidates_processed,
                            total_candidates: prefix_matched_candidates,
                            current_path: candidate.entry.path.clone(),
                        },
                    });
                }

                // Check FileId cache for full hash
                let full_hash_result = if let Some(fid) = candidate.entry.file_id {
                    file_id_full_cache.get(&fid).copied()
                } else {
                    None
                };

                let full_hash = match full_hash_result {
                    Some(cached) => cached,
                    None => {
                        match stream_full_hash(
                            &candidate.entry.path,
                            candidate.entry.size,
                            options.cancellation_token.as_ref(),
                        ) {
                            Ok(hash) => {
                                if let Some(fid) = candidate.entry.file_id {
                                    file_id_full_cache.insert(fid, hash);
                                }
                                hash
                            }
                            Err(err) => {
                                errors.push(DuplicateFileError {
                                    path: candidate.entry.path.clone(),
                                    message: format!("Failed to compute full hash: {}", err),
                                });
                                continue;
                            }
                        }
                    }
                };

                local_full_map
                    .entry(full_hash)
                    .or_default()
                    .push(candidate.entry);
            }

            for (hash, files) in local_full_map {
                if files.len() >= 2 {
                    verified_groups.insert((file_size, hash), files);
                }
            }
        }
    }

    // Step 4: Construct DuplicateGroup instances with hard-link awareness
    let mut duplicate_groups: Vec<DuplicateGroup> = Vec::new();

    for ((file_size, hash), mut group_entries) in verified_groups {
        // Sort entries deterministically by path
        group_entries.sort_by(|a, b| a.path.cmp(&b.path));

        // Group paths into PhysicalCopy by FileId
        let mut physical_copies: Vec<PhysicalCopy> = Vec::new();
        let mut file_id_map: HashMap<FileId, usize> = HashMap::new();

        for entry in &group_entries {
            match entry.file_id {
                Some(fid) => {
                    if let Some(&idx) = file_id_map.get(&fid) {
                        physical_copies[idx].paths.push(entry.path.clone());
                    } else {
                        let idx = physical_copies.len();
                        file_id_map.insert(fid, idx);
                        physical_copies.push(PhysicalCopy {
                            file_id: Some(fid),
                            paths: vec![entry.path.clone()],
                        });
                    }
                }
                None => {
                    physical_copies.push(PhysicalCopy {
                        file_id: None,
                        paths: vec![entry.path.clone()],
                    });
                }
            }
        }

        // Sort paths inside each physical copy for deterministic presentation
        for copy in &mut physical_copies {
            copy.paths.sort();
        }

        // Sort physical copies by their first path
        physical_copies.sort_by(|a, b| a.paths[0].cmp(&b.paths[0]));

        let distinct_physical_copies = physical_copies.len();
        let reclaimable_bytes = if distinct_physical_copies > 1 {
            (distinct_physical_copies as u64 - 1) * file_size
        } else {
            0
        };

        duplicate_groups.push(DuplicateGroup {
            hash,
            file_size,
            entries: group_entries.into_iter().cloned().collect(),
            physical_copies,
            distinct_physical_copies,
            reclaimable_bytes,
        });
    }

    // Sort duplicate groups by reclaimable bytes descending, then file size descending, then hash
    duplicate_groups.sort_by(|a, b| {
        b.reclaimable_bytes
            .cmp(&a.reclaimable_bytes)
            .then_with(|| b.file_size.cmp(&a.file_size))
            .then_with(|| a.hash.as_bytes().cmp(b.hash.as_bytes()))
    });

    // Summary calculations
    let duplicate_groups_count = duplicate_groups.len();
    let duplicate_files_count: usize = duplicate_groups.iter().map(|g| g.entries.len()).sum();
    let total_distinct_copies: usize = duplicate_groups
        .iter()
        .map(|g| g.distinct_physical_copies)
        .sum();
    let hard_link_aliases_count = duplicate_files_count.saturating_sub(total_distinct_copies);
    let total_reclaimable_bytes: u64 = duplicate_groups.iter().map(|g| g.reclaimable_bytes).sum();

    DuplicateOutput {
        groups: duplicate_groups,
        summary: DuplicateSummary {
            total_files_evaluated: entries.len() as u64,
            empty_files_skipped,
            size_matched_candidates,
            prefix_matched_candidates,
            duplicate_groups_count,
            duplicate_files_count,
            distinct_physical_copies: total_distinct_copies,
            hard_link_aliases_count,
            total_reclaimable_bytes,
            elapsed: start_time.elapsed(),
            was_cancelled,
        },
        errors,
    }
}

/// Reads up to `prefix_limit` bytes from a file and computes its BLAKE3 hash.
///
/// Returns `(hash, is_full_hash)` where `is_full_hash` is `true` if the entire file
/// content was read (i.e. `expected_size <= prefix_limit`).
fn read_prefix_hash(
    path: &Path,
    expected_size: u64,
    prefix_limit: usize,
) -> Result<(blake3::Hash, bool), std::io::Error> {
    let mut file = File::open(path)?;
    let bytes_to_read = (expected_size as usize).min(prefix_limit);
    let mut buffer = vec![0u8; bytes_to_read];

    let mut total_read = 0;
    while total_read < bytes_to_read {
        let n = file.read(&mut buffer[total_read..])?;
        if n == 0 {
            break;
        }
        total_read += n;
    }

    if total_read != bytes_to_read {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!(
                "file size changed during prefix read (expected {} bytes, read {})",
                bytes_to_read, total_read
            ),
        ));
    }

    // Defensive check: if the file was expected to be <= prefix_limit, ensure no extra bytes exist
    let is_full = expected_size <= prefix_limit as u64;
    if is_full {
        let mut check_extra = [0u8; 1];
        if file.read(&mut check_extra)? != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "file size changed during read (file grew beyond {} bytes)",
                    expected_size
                ),
            ));
        }
    }

    let hash = blake3::hash(&buffer);
    Ok((hash, is_full))
}

/// Streams the entire content of a file through a BLAKE3 hasher using a bounded buffer.
///
/// Checks cancellation periodically after every buffer read.
fn stream_full_hash(
    path: &Path,
    expected_size: u64,
    token: Option<&CancellationToken>,
) -> Result<blake3::Hash, std::io::Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(HASH_BUFFER_SIZE, file);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; HASH_BUFFER_SIZE];
    let mut total_read = 0u64;

    loop {
        if let Some(t) = token {
            if t.is_cancelled() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "duplicate detection cancelled",
                ));
            }
        }

        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        total_read += n as u64;
    }

    if total_read != expected_size {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "file size changed during full hash (expected {} bytes, read {})",
                expected_size, total_read
            ),
        ));
    }

    Ok(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_hash_equals_full_hash_for_small_file() {
        let content = b"Reclaim safe duplicate detection test content";
        let full_hash = blake3::hash(content);

        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("reclaim_test_prefix_eq_full.tmp");
        std::fs::write(&path, content).expect("write temp file");

        let (prefix_hash, is_full) =
            read_prefix_hash(&path, content.len() as u64, 16384).expect("read prefix");
        assert!(is_full);
        assert_eq!(prefix_hash, full_hash);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_duplicate_group_properties() {
        let hash = blake3::hash(b"test");
        let group_no_links = DuplicateGroup {
            hash,
            file_size: 100,
            entries: vec![
                FileEntry {
                    path: PathBuf::from("/a"),
                    size: 100,
                    modified: None,
                    file_id: Some(FileId::Unix { dev: 1, ino: 1 }),
                    readonly: false,
                },
                FileEntry {
                    path: PathBuf::from("/b"),
                    size: 100,
                    modified: None,
                    file_id: Some(FileId::Unix { dev: 1, ino: 2 }),
                    readonly: false,
                },
            ],
            physical_copies: vec![
                PhysicalCopy {
                    file_id: Some(FileId::Unix { dev: 1, ino: 1 }),
                    paths: vec![PathBuf::from("/a")],
                },
                PhysicalCopy {
                    file_id: Some(FileId::Unix { dev: 1, ino: 2 }),
                    paths: vec![PathBuf::from("/b")],
                },
            ],
            distinct_physical_copies: 2,
            reclaimable_bytes: 100,
        };

        assert_eq!(group_no_links.file_count(), 2);
        assert!(!group_no_links.has_hard_links());
        assert!(!group_no_links.is_pure_hard_links());
        assert_eq!(group_no_links.reclaimable_bytes, 100);

        let group_pure_links = DuplicateGroup {
            hash,
            file_size: 100,
            entries: vec![
                FileEntry {
                    path: PathBuf::from("/a"),
                    size: 100,
                    modified: None,
                    file_id: Some(FileId::Unix { dev: 1, ino: 1 }),
                    readonly: false,
                },
                FileEntry {
                    path: PathBuf::from("/b"),
                    size: 100,
                    modified: None,
                    file_id: Some(FileId::Unix { dev: 1, ino: 1 }),
                    readonly: false,
                },
            ],
            physical_copies: vec![PhysicalCopy {
                file_id: Some(FileId::Unix { dev: 1, ino: 1 }),
                paths: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            }],
            distinct_physical_copies: 1,
            reclaimable_bytes: 0,
        };

        assert_eq!(group_pure_links.file_count(), 2);
        assert!(group_pure_links.has_hard_links());
        assert!(group_pure_links.is_pure_hard_links());
        assert_eq!(group_pure_links.reclaimable_bytes, 0);
    }
}
