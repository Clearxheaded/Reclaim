//! # Cleanup Planner Module
//!
//! Provides prospective, immutable cleanup plan generation for duplicate files (Milestone 3).
//!
//! ## Safety Invariants
//! - **Purely Read-Only**: Planning never modifies, deletes, moves, or creates files on the filesystem.
//! - **Never Quarantine a Keeper**: Preserved keeper files are never targeted by quarantine actions.
//! - **Verified Keeper Requirement**: No quarantine action is ever proposed unless a verified keeper
//!   holding the exact same content exists in the plan.
//! - **Collision Prevention**: Every proposed quarantine destination is validated against collisions
//!   with other proposed destinations, sources, and keepers.
//! - **TOCTOU Defense via Preconditions**: Every proposed action records immutable preconditions
//!   (expected size, BLAKE3 content hash, mtime, and `FileId`) that must be re-checked before execution.
//! - **Explicit Hard-Link Awareness**: Hard-link aliases sharing physical storage with the keeper are
//!   preserved and protected. Pure hard-link duplicate groups yield zero quarantine actions and zero
//!   reclaimable bytes.

use crate::duplicates::DuplicateGroup;
use crate::entry::FileEntry;
use crate::file_id::FileId;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Strategy used to choose which file in a duplicate group should be preserved as the "keeper".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeeperPolicy {
    /// Prefer paths with the shortest depth (fewest path components) and shortest string length.
    ShortestPath,
    /// Prefer files with the oldest modification time (earliest mtime).
    OldestModified,
    /// Prefer files with the newest modification time (latest mtime).
    NewestModified,
    /// Prefer files residing under a designated directory tree, falling back to `ShortestPath`.
    PreferredDirectory(PathBuf),
    /// Prefer files residing under a designated directory tree, falling back to the specified strategy.
    PreferredDirectoryWithStrategy {
        /// The canonical directory tree to prioritize.
        preferred_dir: PathBuf,
        /// Fallback strategy when multiple candidates (or no candidates) reside in `preferred_dir`.
        strategy: Box<KeeperPolicy>,
    },
}

impl KeeperPolicy {
    /// Compares two candidate file entries under this policy.
    ///
    /// Returns `std::cmp::Ordering::Less` if `a` is preferred over `b` as keeper.
    /// This comparison is strictly total and deterministic for distinct paths.
    pub fn cmp_entries(&self, a: &FileEntry, b: &FileEntry) -> std::cmp::Ordering {
        match self {
            KeeperPolicy::ShortestPath => Self::cmp_shortest_path(a, b),
            KeeperPolicy::OldestModified => match (&a.modified, &b.modified) {
                (Some(time_a), Some(time_b)) => {
                    let ord = time_a.cmp(time_b);
                    if ord != std::cmp::Ordering::Equal {
                        ord
                    } else {
                        Self::cmp_shortest_path(a, b)
                    }
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => Self::cmp_shortest_path(a, b),
            },
            KeeperPolicy::NewestModified => {
                match (&a.modified, &b.modified) {
                    (Some(time_a), Some(time_b)) => {
                        let ord = time_b.cmp(time_a); // later time is preferred
                        if ord != std::cmp::Ordering::Equal {
                            ord
                        } else {
                            Self::cmp_shortest_path(a, b)
                        }
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => Self::cmp_shortest_path(a, b),
                }
            }
            KeeperPolicy::PreferredDirectory(preferred_dir) => {
                Self::cmp_preferred_dir(preferred_dir, &KeeperPolicy::ShortestPath, a, b)
            }
            KeeperPolicy::PreferredDirectoryWithStrategy {
                preferred_dir,
                strategy,
            } => Self::cmp_preferred_dir(preferred_dir, strategy, a, b),
        }
    }

    /// Selects the best keeper from a slice of candidate file entries.
    pub fn select_keeper<'a>(&self, entries: &'a [FileEntry]) -> Option<&'a FileEntry> {
        entries.iter().min_by(|a, b| self.cmp_entries(a, b))
    }

    /// Explains why the keeper was chosen under this policy.
    pub fn explain_selection(&self, keeper: &FileEntry, is_tie_break: bool) -> String {
        let tie_suffix = if is_tie_break {
            " (selected by deterministic tie-break)"
        } else {
            ""
        };

        match self {
            KeeperPolicy::ShortestPath => {
                let depth = keeper.path.components().count();
                format!(
                    "Preserved shortest path (depth {}, {} characters){}",
                    depth,
                    keeper.path.as_os_str().len(),
                    tie_suffix
                )
            }
            KeeperPolicy::OldestModified => {
                if let Some(mtime) = keeper.modified {
                    format!(
                        "Preserved oldest modification timestamp ({:?}){}",
                        mtime, tie_suffix
                    )
                } else {
                    format!("Preserved under OldestModified policy{}", tie_suffix)
                }
            }
            KeeperPolicy::NewestModified => {
                if let Some(mtime) = keeper.modified {
                    format!(
                        "Preserved newest modification timestamp ({:?}){}",
                        mtime, tie_suffix
                    )
                } else {
                    format!("Preserved under NewestModified policy{}", tie_suffix)
                }
            }
            KeeperPolicy::PreferredDirectory(dir) => {
                if keeper.path.starts_with(dir) {
                    format!(
                        "Preserved within preferred directory '{}'{}",
                        dir.display(),
                        tie_suffix
                    )
                } else {
                    format!(
                        "Preserved under fallback policy (outside preferred dir '{}'){}",
                        dir.display(),
                        tie_suffix
                    )
                }
            }
            KeeperPolicy::PreferredDirectoryWithStrategy {
                preferred_dir,
                strategy,
            } => {
                if keeper.path.starts_with(preferred_dir) {
                    format!(
                        "Preserved within preferred directory '{}' via {}{}",
                        preferred_dir.display(),
                        strategy.name(),
                        tie_suffix
                    )
                } else {
                    format!(
                        "Preserved via fallback {} (outside preferred dir '{}'){}",
                        strategy.name(),
                        preferred_dir.display(),
                        tie_suffix
                    )
                }
            }
        }
    }

    /// Short human-readable name of the policy.
    pub fn name(&self) -> &'static str {
        match self {
            KeeperPolicy::ShortestPath => "ShortestPath",
            KeeperPolicy::OldestModified => "OldestModified",
            KeeperPolicy::NewestModified => "NewestModified",
            KeeperPolicy::PreferredDirectory(_) => "PreferredDirectory",
            KeeperPolicy::PreferredDirectoryWithStrategy { .. } => "PreferredDirectoryWithStrategy",
        }
    }

    fn cmp_shortest_path(a: &FileEntry, b: &FileEntry) -> std::cmp::Ordering {
        // 1. Path depth (fewest components)
        let depth_a = a.path.components().count();
        let depth_b = b.path.components().count();
        if depth_a != depth_b {
            return depth_a.cmp(&depth_b);
        }

        // 2. String length (shorter path string)
        let len_a = a.path.as_os_str().len();
        let len_b = b.path.as_os_str().len();
        if len_a != len_b {
            return len_a.cmp(&len_b);
        }

        // 3. Strict lexicographical tie-breaker for deterministic repeatability
        a.path.cmp(&b.path)
    }

    fn cmp_preferred_dir(
        preferred_dir: &Path,
        fallback: &KeeperPolicy,
        a: &FileEntry,
        b: &FileEntry,
    ) -> std::cmp::Ordering {
        let in_a = a.path.starts_with(preferred_dir);
        let in_b = b.path.starts_with(preferred_dir);

        match (in_a, in_b) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => fallback.cmp_entries(a, b),
        }
    }
}

/// Preconditions that must be re-validated at execution time before any mutation occurs.
///
/// Defends against Time-of-Check / Time-of-Use (TOCTOU) races where files are modified,
/// replaced with symlinks, or deleted between planning and execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPreconditions {
    /// Expected byte length of the file.
    pub expected_size: u64,
    /// Expected BLAKE3 cryptographic content hash in hex encoding.
    pub expected_hash: String,
    /// Expected modification time of the source file, if known.
    pub source_modified: Option<SystemTime>,
    /// Expected physical identity (`FileId`) of the source file, if supported.
    pub source_file_id: Option<FileId>,
    /// Expected modification time of the preserved keeper file, if known.
    pub keeper_modified: Option<SystemTime>,
    /// Expected physical identity (`FileId`) of the preserved keeper file, if supported.
    pub keeper_file_id: Option<FileId>,
}

impl ActionPreconditions {
    /// Parses the recorded hex hash back into a `blake3::Hash`.
    pub fn expected_blake3_hash(&self) -> Result<blake3::Hash, blake3::HexError> {
        blake3::Hash::from_hex(&self.expected_hash)
    }
}

/// An immutable, prospective action proposing to quarantine a verified duplicate file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantineAction {
    /// Path of the source file to be quarantined.
    pub source_path: PathBuf,
    /// Staged destination path in the quarantine directory.
    pub quarantine_path: PathBuf,
    /// Path of the verified keeper file preserving this content.
    pub keeper_path: PathBuf,
    /// Concrete preconditions required for safe execution.
    pub preconditions: ActionPreconditions,
    /// Human-readable explanation of why this action was proposed.
    pub rationale: String,
}

impl QuarantineAction {
    /// Returns the expected byte length of the source file.
    pub fn expected_size(&self) -> u64 {
        self.preconditions.expected_size
    }

    /// Returns the expected BLAKE3 content hash string.
    pub fn expected_hash(&self) -> &str {
        &self.preconditions.expected_hash
    }

    /// Returns the expected source `FileId`.
    pub fn source_file_id(&self) -> Option<FileId> {
        self.preconditions.source_file_id
    }

    /// Returns the expected keeper `FileId`.
    pub fn keeper_file_id(&self) -> Option<FileId> {
        self.preconditions.keeper_file_id
    }
}

/// A preserved file that serves as the canonical copy for a duplicate group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedKeeper {
    /// Full path to the keeper file.
    pub path: PathBuf,
    /// Size of the file in bytes.
    pub size: u64,
    /// BLAKE3 cryptographic hash in hex format.
    pub hash: String,
    /// Filesystem identity (`FileId`), if supported.
    pub file_id: Option<FileId>,
    /// Last modification time, if known.
    pub modified: Option<SystemTime>,
    /// Any hard-link alias paths pointing to this same keeper inode.
    pub hard_link_aliases: Vec<PathBuf>,
    /// Human-readable explanation of why this file was selected as keeper.
    pub rationale: String,
}

/// High-level metrics summarizing a prospective cleanup plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSummary {
    /// Total candidate duplicate files evaluated across all duplicate groups.
    pub total_duplicate_files: usize,
    /// Number of duplicate content groups evaluated.
    pub duplicate_groups_count: usize,
    /// Total number of quarantine actions proposed.
    pub planned_actions: usize,
    /// Total number of protected keeper files.
    pub protected_keepers: usize,
    /// Conservative estimate of storage bytes to be freed upon execution.
    pub total_reclaimable_bytes: u64,
    /// Total files left untouched (keepers + keeper hard-link aliases).
    pub files_left_untouched: usize,
}

/// An immutable, prospective cleanup plan representing reviewable proposed operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupPlan {
    /// Timestamp when this plan was created.
    pub created_at: SystemTime,
    /// The root directory scanned.
    pub root_dir: PathBuf,
    /// The keeper policy used to select preserved files.
    pub policy: KeeperPolicy,
    /// The target quarantine directory where files will be staged.
    pub quarantine_dir: PathBuf,
    /// All proposed quarantine actions.
    pub actions: Vec<QuarantineAction>,
    /// All preserved keeper files with selection rationale.
    pub keepers: Vec<ProtectedKeeper>,
    /// High-level metrics summarizing this plan.
    pub summary: PlanSummary,
}

impl CleanupPlan {
    /// Validates safety invariants across the entire plan.
    ///
    /// Checks for:
    /// - Actions attempting to quarantine their own keeper.
    /// - Destination collisions between actions.
    /// - Actions whose destination collides with an existing keeper or source.
    pub fn validate(&self) -> Result<(), PlanError> {
        let mut seen_destinations: HashMap<&Path, &Path> = HashMap::new();
        let keeper_paths: HashSet<&Path> = self.keepers.iter().map(|k| k.path.as_path()).collect();

        for action in &self.actions {
            if action.source_path == action.keeper_path {
                return Err(PlanError::KeeperTargetCollision {
                    path: action.source_path.clone(),
                });
            }

            if keeper_paths.contains(action.quarantine_path.as_path()) {
                return Err(PlanError::DestinationMatchesKeeper {
                    path: action.quarantine_path.clone(),
                });
            }

            if action.quarantine_path == action.source_path {
                return Err(PlanError::DestinationMatchesSource {
                    path: action.quarantine_path.clone(),
                });
            }

            if let Some(existing_source) = seen_destinations.insert(
                action.quarantine_path.as_path(),
                action.source_path.as_path(),
            ) {
                return Err(PlanError::DestinationCollision {
                    destination: action.quarantine_path.clone(),
                    source_a: existing_source.to_path_buf(),
                    source_b: action.source_path.clone(),
                });
            }
        }

        Ok(())
    }

    /// Serializes this plan to a compact JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serializes this plan to a pretty-printed, human-readable JSON string.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserializes a `CleanupPlan` from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }
}

/// Configuration options for generating a cleanup plan.
#[derive(Debug, Clone)]
pub struct PlanOptions {
    /// Root directory scanned (used for computing quarantine-relative paths).
    pub root_dir: PathBuf,
    /// Strategy for choosing keeper files.
    pub policy: KeeperPolicy,
    /// Target quarantine directory. Defaults to `<root_dir>/.reclaim/quarantine`.
    pub quarantine_dir: Option<PathBuf>,
}

/// Errors that can occur during cleanup plan generation or validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// A duplicate group contained fewer than two entries.
    InvalidGroup {
        /// Description of the invalid group.
        message: String,
    },
    /// Multiple proposed actions target the same destination path.
    DestinationCollision {
        /// The colliding destination path.
        destination: PathBuf,
        /// First source file targeting this destination.
        source_a: PathBuf,
        /// Second source file targeting this destination.
        source_b: PathBuf,
    },
    /// An action attempted to propose quarantining a keeper file.
    KeeperTargetCollision {
        /// The keeper path that was targeted.
        path: PathBuf,
    },
    /// A proposed quarantine destination matches an existing keeper path.
    DestinationMatchesKeeper {
        /// The colliding path.
        path: PathBuf,
    },
    /// A proposed quarantine destination matches the source path itself.
    DestinationMatchesSource {
        /// The colliding path.
        path: PathBuf,
    },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::InvalidGroup { message } => {
                write!(f, "invalid duplicate group: {}", message)
            }
            PlanError::DestinationCollision {
                destination,
                source_a,
                source_b,
            } => {
                write!(
                    f,
                    "quarantine destination collision: '{}' is targeted by both '{}' and '{}'",
                    destination.display(),
                    source_a.display(),
                    source_b.display()
                )
            }
            PlanError::KeeperTargetCollision { path } => {
                write!(
                    f,
                    "unsafe action: attempted to quarantine keeper file '{}'",
                    path.display()
                )
            }
            PlanError::DestinationMatchesKeeper { path } => {
                write!(
                    f,
                    "quarantine destination '{}' collides with protected keeper path",
                    path.display()
                )
            }
            PlanError::DestinationMatchesSource { path } => {
                write!(
                    f,
                    "quarantine destination '{}' collides with source path",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// Generates an immutable, reviewable cleanup plan from verified duplicate groups.
///
/// This operation is strictly read-only and performs zero filesystem mutations.
pub fn generate_plan(
    duplicate_groups: &[DuplicateGroup],
    options: &PlanOptions,
) -> Result<CleanupPlan, PlanError> {
    let quarantine_dir = options
        .quarantine_dir
        .clone()
        .unwrap_or_else(|| options.root_dir.join(".reclaim").join("quarantine"));

    let mut actions = Vec::new();
    let mut keepers = Vec::new();
    let mut total_duplicate_files = 0;
    let mut total_reclaimable_bytes = 0;
    let mut files_left_untouched = 0;

    // Track assigned destinations to detect collision risks early during generation
    let mut seen_destinations: HashMap<PathBuf, PathBuf> = HashMap::new();

    for group in duplicate_groups {
        if group.entries.len() < 2 {
            return Err(PlanError::InvalidGroup {
                message: format!(
                    "group with hash {} has {} entries; at least 2 entries required",
                    group.hash,
                    group.entries.len()
                ),
            });
        }

        total_duplicate_files += group.entries.len();

        // 1. Select the keeper using the requested policy
        let keeper_entry = options
            .policy
            .select_keeper(&group.entries)
            .expect("group has at least 2 entries");

        let keeper_hash_hex = group.hash.to_hex().to_string();

        // 2. Identify keeper hard-link aliases (entries sharing keeper's physical FileId)
        let mut keeper_aliases = Vec::new();
        let mut candidate_entries = Vec::new();

        for entry in &group.entries {
            if entry.path == keeper_entry.path {
                continue;
            }

            if let (Some(k_id), Some(e_id)) = (keeper_entry.file_id, entry.file_id) {
                if k_id == e_id {
                    // This entry is a hard link to the keeper itself!
                    // Removing it does not free physical disk storage and breaks an alias to the keeper.
                    keeper_aliases.push(entry.path.clone());
                    continue;
                }
            }

            candidate_entries.push(entry);
        }

        // Check if there was a tie when choosing the keeper
        let is_tie = group.entries.iter().any(|e| {
            e.path != keeper_entry.path
                && options.policy.cmp_entries(e, keeper_entry) == std::cmp::Ordering::Equal
        });

        let keeper_rationale = options.policy.explain_selection(keeper_entry, is_tie);

        let protected_keeper = ProtectedKeeper {
            path: keeper_entry.path.clone(),
            size: keeper_entry.size,
            hash: keeper_hash_hex.clone(),
            file_id: keeper_entry.file_id,
            modified: keeper_entry.modified,
            hard_link_aliases: keeper_aliases.clone(),
            rationale: keeper_rationale,
        };

        keepers.push(protected_keeper);
        files_left_untouched += 1 + keeper_aliases.len();

        // 3. If candidate entries exist (distinct physical copies), propose quarantine actions
        if !candidate_entries.is_empty() {
            total_reclaimable_bytes += group.reclaimable_bytes;

            for entry in candidate_entries {
                // Determine quarantine destination path
                let quarantine_path =
                    compute_quarantine_path(&entry.path, &options.root_dir, &quarantine_dir);

                // Prevent collision risks where multiple actions target the same destination
                if let Some(existing_source) =
                    seen_destinations.insert(quarantine_path.clone(), entry.path.clone())
                {
                    return Err(PlanError::DestinationCollision {
                        destination: quarantine_path,
                        source_a: existing_source,
                        source_b: entry.path.clone(),
                    });
                }

                let rationale = format!(
                    "Duplicate copy of preserved keeper '{}' under policy '{}'",
                    keeper_entry.path.display(),
                    options.policy.name()
                );

                let preconditions = ActionPreconditions {
                    expected_size: entry.size,
                    expected_hash: keeper_hash_hex.clone(),
                    source_modified: entry.modified,
                    source_file_id: entry.file_id,
                    keeper_modified: keeper_entry.modified,
                    keeper_file_id: keeper_entry.file_id,
                };

                actions.push(QuarantineAction {
                    source_path: entry.path.clone(),
                    quarantine_path,
                    keeper_path: keeper_entry.path.clone(),
                    preconditions,
                    rationale,
                });
            }
        }
    }

    let summary = PlanSummary {
        total_duplicate_files,
        duplicate_groups_count: duplicate_groups.len(),
        planned_actions: actions.len(),
        protected_keepers: keepers.len(),
        total_reclaimable_bytes,
        files_left_untouched,
    };

    let plan = CleanupPlan {
        created_at: SystemTime::now(),
        root_dir: options.root_dir.clone(),
        policy: options.policy.clone(),
        quarantine_dir,
        actions,
        keepers,
        summary,
    };

    // Perform final validation check before returning the plan
    plan.validate()?;

    Ok(plan)
}

/// Derives a clean staged destination in the quarantine directory.
///
/// Preserves relative subdirectories from the scan root to maintain directory structure
/// and prevent filename collisions between files with identical names in different folders.
fn compute_quarantine_path(source_path: &Path, root_dir: &Path, quarantine_dir: &Path) -> PathBuf {
    if let Ok(rel_path) = source_path.strip_prefix(root_dir) {
        quarantine_dir.join(rel_path)
    } else {
        // Fallback: if not under root, use file name
        let file_name = source_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("quarantined_file"));
        quarantine_dir.join(file_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_entry(
        path: &str,
        size: u64,
        mtime_offset_secs: u64,
        file_id: Option<FileId>,
    ) -> FileEntry {
        let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        FileEntry {
            path: PathBuf::from(path),
            size,
            modified: Some(base_time + Duration::from_secs(mtime_offset_secs)),
            file_id,
            readonly: false,
        }
    }

    #[test]
    fn test_shortest_path_policy() {
        let policy = KeeperPolicy::ShortestPath;

        let a = make_entry("/root/file.txt", 100, 10, None);
        let b = make_entry("/root/nested/dir/file.txt", 100, 5, None);

        // a has 2 components, b has 4 components => a is preferred
        assert_eq!(policy.cmp_entries(&a, &b), std::cmp::Ordering::Less);
        assert_eq!(policy.cmp_entries(&b, &a), std::cmp::Ordering::Greater);
    }

    #[test]
    fn test_oldest_modified_policy() {
        let policy = KeeperPolicy::OldestModified;

        let older = make_entry("/root/sub/older.txt", 100, 10, None);
        let newer = make_entry("/root/newer.txt", 100, 20, None);

        // older has mtime 10, newer has mtime 20 => older is preferred
        assert_eq!(policy.cmp_entries(&older, &newer), std::cmp::Ordering::Less);
        assert_eq!(
            policy.cmp_entries(&newer, &older),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_newest_modified_policy() {
        let policy = KeeperPolicy::NewestModified;

        let older = make_entry("/root/older.txt", 100, 10, None);
        let newer = make_entry("/root/sub/newer.txt", 100, 20, None);

        // newer is preferred under NewestModified
        assert_eq!(policy.cmp_entries(&newer, &older), std::cmp::Ordering::Less);
        assert_eq!(
            policy.cmp_entries(&older, &newer),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_preferred_directory_policy() {
        let preferred_dir = PathBuf::from("/canonical/library");
        let policy = KeeperPolicy::PreferredDirectory(preferred_dir);

        let preferred = make_entry("/canonical/library/photo.jpg", 100, 20, None);
        let external = make_entry("/downloads/photo.jpg", 100, 5, None);

        // preferred is inside /canonical/library, external is not => preferred wins
        assert_eq!(
            policy.cmp_entries(&preferred, &external),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            policy.cmp_entries(&external, &preferred),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_deterministic_tie_break() {
        let policy = KeeperPolicy::ShortestPath;

        let a = make_entry("/root/a.txt", 100, 10, None);
        let b = make_entry("/root/b.txt", 100, 10, None);

        // Same depth, same length: 'a.txt' < 'b.txt' lexicographically
        assert_eq!(policy.cmp_entries(&a, &b), std::cmp::Ordering::Less);
        assert_eq!(policy.cmp_entries(&b, &a), std::cmp::Ordering::Greater);
    }
}
