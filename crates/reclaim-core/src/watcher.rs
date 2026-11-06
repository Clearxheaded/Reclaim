//! # Real-Time Filesystem Change Detection & Active TOCTOU Defense
//!
//! Streams kernel-level filesystem events (`FSEvents` on macOS Darwin,
//! `ReadDirectoryChangesW` on Windows, `inotify` on Linux) to monitor scanned
//! directories while the user inspects candidate duplicate files.
//!
//! ## Active TOCTOU (Time-of-Check to Time-of-Use) Defense
//! Traditional duplicate cleaners suffer from TOCTOU race conditions: between the time
//! a directory is scanned and the time the user approves a cleanup plan (which may be
//! minutes or hours), external processes or users may edit, delete, or rename files.
//!
//! This module continuously streams OS filesystem events to:
//! 1. Instantly detect modifications, deletions, and moves of scanned candidates.
//! 2. Invalidate affected candidate files before any cleanup plan is staged.
//! 3. Dynamically re-evaluate duplicate groups and reclaimable storage in real time.
//! 4. Emit notifications to user interfaces and execution engines.

use crate::duplicates::{DuplicateGroup, DuplicateOutput, DuplicateSummary};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// High-level domain filesystem change event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FsChangeEvent {
    /// File content or size was modified on disk.
    Modified(PathBuf),
    /// File or parent directory was deleted from disk.
    Removed(PathBuf),
    /// File or directory was moved / renamed.
    Renamed { from: PathBuf, to: PathBuf },
    /// General directory mutation requiring re-verification.
    RescanRequired(PathBuf),
}

/// Specific reason why a candidate duplicate was invalidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvalidationReason {
    /// File was modified on disk after scan completed.
    Modified,
    /// File was deleted on disk.
    Removed,
    /// File was moved or renamed to another path.
    Renamed,
}

impl fmt::Display for InvalidationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidationReason::Modified => write!(f, "File modified on disk"),
            InvalidationReason::Removed => write!(f, "File deleted from disk"),
            InvalidationReason::Renamed => write!(f, "File moved or renamed"),
        }
    }
}

/// Notification emitted when a scanned candidate is invalidated by a filesystem event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationNotice {
    /// The invalid file path.
    pub path: PathBuf,
    /// Why the candidate was invalidated.
    pub reason: InvalidationReason,
    /// BLAKE3 hex hash of the duplicate group affected.
    pub group_hash: String,
    /// Number of duplicate files remaining in the group after invalidation.
    pub remaining_files_in_group: usize,
    /// Recalculated total reclaimable bytes across all groups.
    pub new_reclaimable_bytes: u64,
}

/// Errors originating from filesystem watcher initialization or monitoring.
#[derive(Debug)]
pub enum WatcherError {
    /// Underlying notify initialization failure.
    Notify(notify::Error),
    /// Root path could not be resolved or watched.
    WatchPathFailed { path: PathBuf, error: String },
}

impl fmt::Display for WatcherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatcherError::Notify(err) => write!(f, "Filesystem watcher error: {}", err),
            WatcherError::WatchPathFailed { path, error } => {
                write!(f, "Failed to watch path '{}': {}", path.display(), error)
            }
        }
    }
}

impl std::error::Error for WatcherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WatcherError::Notify(err) => Some(err),
            _ => None,
        }
    }
}

impl From<notify::Error> for WatcherError {
    fn from(err: notify::Error) -> Self {
        WatcherError::Notify(err)
    }
}

/// Tracks candidate duplicate files and applies filesystem change events to invalidate
/// stale candidates in real time.
#[derive(Debug, Clone)]
pub struct CandidateInvalidator {
    /// Map of canonical file path -> BLAKE3 hex hash.
    path_to_hash: HashMap<PathBuf, String>,
    /// Active duplicate groups indexed by BLAKE3 hex hash.
    groups: HashMap<String, DuplicateGroup>,
    /// Scanned directories being actively tracked.
    roots: Vec<PathBuf>,
    /// Underlying summary template.
    summary: DuplicateSummary,
}

impl CandidateInvalidator {
    /// Constructs an invalidator initialized from an active `DuplicateOutput` and roots.
    pub fn new(output: DuplicateOutput, roots: Vec<PathBuf>) -> Self {
        let mut path_to_hash = HashMap::new();
        let mut groups = HashMap::new();

        for group in &output.groups {
            let hash_str = group.hash.to_hex().to_string();
            for entry in &group.entries {
                path_to_hash.insert(entry.path.clone(), hash_str.clone());
                if let Ok(canon) = std::fs::canonicalize(&entry.path) {
                    if canon != entry.path {
                        path_to_hash.insert(canon, hash_str.clone());
                    }
                }
            }
            groups.insert(hash_str, group.clone());
        }

        Self {
            path_to_hash,
            groups,
            roots,
            summary: output.summary,
        }
    }

    /// Checks whether a given path is currently tracked as a candidate duplicate.
    pub fn is_candidate(&self, path: &Path) -> bool {
        self.path_to_hash.contains_key(path)
    }

    /// Returns the number of currently tracked candidates.
    pub fn candidate_count(&self) -> usize {
        self.groups.values().map(|g| g.entries.len()).sum()
    }

    /// Invalidates a specific candidate path, updating groups and reclaimable storage.
    pub fn invalidate_path(
        &mut self,
        path: &Path,
        reason: InvalidationReason,
    ) -> Option<InvalidationNotice> {
        let group_hash = match self.path_to_hash.remove(path) {
            Some(h) => h,
            None => {
                let canon_hash = std::fs::canonicalize(path)
                    .ok()
                    .and_then(|c| self.path_to_hash.remove(&c));

                if let Some(h) = canon_hash {
                    h
                } else {
                    // Check if path is an ancestor or directory containing tracked files
                    let matching_paths: Vec<PathBuf> = self
                        .path_to_hash
                        .keys()
                        .filter(|p| p.starts_with(path))
                        .cloned()
                        .collect();

                    if matching_paths.is_empty() {
                        return None;
                    }

                    let mut last_notice = None;
                    for p in matching_paths {
                        if let Some(notice) = self.invalidate_path(&p, reason) {
                            last_notice = Some(notice);
                        }
                    }
                    return last_notice;
                }
            }
        };

        let mut group_dissolved = false;
        let mut remaining_files_in_group = 0;

        if let Some(group) = self.groups.get_mut(&group_hash) {
            // Remove entry matching path or canonical equivalent
            group.entries.retain(|e| {
                e.path != path
                    && std::fs::canonicalize(&e.path).ok().as_deref() != Some(path)
                    && Some(e.path.as_path()) != std::fs::canonicalize(path).ok().as_deref()
            });

            // Update physical copies
            for pc in &mut group.physical_copies {
                pc.paths.retain(|p| {
                    p != path
                        && std::fs::canonicalize(p).ok().as_deref() != Some(path)
                        && Some(p.as_path()) != std::fs::canonicalize(path).ok().as_deref()
                });
            }
            group.physical_copies.retain(|pc| !pc.paths.is_empty());
            group.distinct_physical_copies = group.physical_copies.len();

            // Recalculate group reclaimable bytes
            if group.distinct_physical_copies > 1 {
                group.reclaimable_bytes =
                    ((group.distinct_physical_copies - 1) as u64) * group.file_size;
            } else {
                group.reclaimable_bytes = 0;
            }

            remaining_files_in_group = group.entries.len();

            // A group with <= 1 file is no longer a duplicate group
            if group.entries.len() <= 1 {
                group_dissolved = true;
            }
        }

        if group_dissolved {
            if let Some(dissolved) = self.groups.remove(&group_hash) {
                // Unregister any remaining entries from index
                for entry in dissolved.entries {
                    self.path_to_hash.remove(&entry.path);
                }
            }
            remaining_files_in_group = 0;
        }

        // Update summary metrics
        self.summary.duplicate_groups_count = self.groups.len();
        self.summary.duplicate_files_count = self.groups.values().map(|g| g.entries.len()).sum();
        self.summary.distinct_physical_copies = self
            .groups
            .values()
            .map(|g| g.distinct_physical_copies)
            .sum();
        self.summary.total_reclaimable_bytes =
            self.groups.values().map(|g| g.reclaimable_bytes).sum();

        Some(InvalidationNotice {
            path: path.to_path_buf(),
            reason,
            group_hash,
            remaining_files_in_group,
            new_reclaimable_bytes: self.summary.total_reclaimable_bytes,
        })
    }

    /// Ingests a filesystem change event and invalidates any affected candidates.
    pub fn apply_event(&mut self, event: &FsChangeEvent) -> Vec<InvalidationNotice> {
        let mut notices = Vec::new();

        match event {
            FsChangeEvent::Modified(path) => {
                if let Some(notice) = self.invalidate_path(path, InvalidationReason::Modified) {
                    notices.push(notice);
                }
            }
            FsChangeEvent::Removed(path) => {
                if let Some(notice) = self.invalidate_path(path, InvalidationReason::Removed) {
                    notices.push(notice);
                }
            }
            FsChangeEvent::Renamed { from, to: _ } => {
                if let Some(notice) = self.invalidate_path(from, InvalidationReason::Renamed) {
                    notices.push(notice);
                }
            }
            FsChangeEvent::RescanRequired(path) => {
                if let Some(notice) = self.invalidate_path(path, InvalidationReason::Modified) {
                    notices.push(notice);
                }
            }
        }

        notices
    }

    /// Computes total reclaimable bytes across all active duplicate groups.
    pub fn calculate_total_reclaimable_bytes(&self) -> u64 {
        self.summary.total_reclaimable_bytes
    }

    /// Returns a fresh, up-to-date `DuplicateOutput` reflecting real-time invalidations.
    pub fn current_output(&self) -> DuplicateOutput {
        let mut groups: Vec<DuplicateGroup> = self.groups.values().cloned().collect();
        // Deterministic sorting by reclaimable bytes descending, then size descending
        groups.sort_by(|a, b| {
            b.reclaimable_bytes
                .cmp(&a.reclaimable_bytes)
                .then_with(|| b.file_size.cmp(&a.file_size))
        });

        DuplicateOutput {
            groups,
            summary: self.summary.clone(),
            errors: Vec::new(),
        }
    }

    /// Returns the monitored root directories.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }
}

/// Active filesystem watcher streaming OS events and invalidating duplicate candidates in real time.
pub struct ActiveFsWatcher {
    _watcher: RecommendedWatcher,
    event_receiver: Receiver<FsChangeEvent>,
    invalidator: Arc<Mutex<CandidateInvalidator>>,
}

impl ActiveFsWatcher {
    /// Spawns a kernel-level filesystem event stream on `roots`, tracking candidates from `output`.
    pub fn start(output: DuplicateOutput, roots: Vec<PathBuf>) -> Result<Self, WatcherError> {
        let invalidator = Arc::new(Mutex::new(CandidateInvalidator::new(output, roots.clone())));
        let (tx, rx) = channel::<FsChangeEvent>();

        let tx_clone = tx.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    translate_notify_event(event, &tx_clone);
                }
            },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        )?;

        for root in &roots {
            let watch_path = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            if watch_path.exists() {
                watcher
                    .watch(&watch_path, RecursiveMode::Recursive)
                    .map_err(|e| WatcherError::WatchPathFailed {
                        path: root.clone(),
                        error: e.to_string(),
                    })?;
            }
        }

        Ok(Self {
            _watcher: watcher,
            event_receiver: rx,
            invalidator,
        })
    }

    /// Polls pending kernel filesystem events, updating tracked candidate state
    /// and returning all generated invalidation notices.
    pub fn poll_invalidations(&self) -> Vec<InvalidationNotice> {
        let mut notices = Vec::new();
        while let Ok(event) = self.event_receiver.try_recv() {
            if let Ok(mut inv) = self.invalidator.lock() {
                let batch = inv.apply_event(&event);
                notices.extend(batch);
            }
        }
        notices
    }

    /// Returns the thread-safe reference to the current candidate invalidator.
    pub fn invalidator(&self) -> Arc<Mutex<CandidateInvalidator>> {
        Arc::clone(&self.invalidator)
    }
}

/// Translates raw `notify::Event` into structured domain `FsChangeEvent`.
fn translate_notify_event(event: Event, tx: &Sender<FsChangeEvent>) {
    match event.kind {
        EventKind::Modify(_) => {
            for path in event.paths {
                let _ = tx.send(FsChangeEvent::Modified(path));
            }
        }
        EventKind::Remove(_) => {
            for path in event.paths {
                let _ = tx.send(FsChangeEvent::Removed(path));
            }
        }
        EventKind::Create(_) | EventKind::Access(_) => {
            // New files and read accesses do not invalidate existing duplicate hashes
        }
        EventKind::Any | EventKind::Other => {
            for path in event.paths {
                let _ = tx.send(FsChangeEvent::RescanRequired(path));
            }
        }
    }
}
